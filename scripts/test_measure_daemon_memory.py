"""Regression checks for the unprivileged measurement driver's failure paths."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure-daemon-memory.py"))
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)


def stage(name, ms=0, utime=0, stime=0, threads=2, fds=4, rss=100, pss=50):
    return {"stage": name,
            "clock": {"elapsed_ms": ms, "cpu_utime_ticks": utime, "cpu_stime_ticks": stime},
            "process_kib": {"Rss": rss, "Pss": pss},
            "process_counts": {"threads": threads, "fds": fds}}


def marker_text(stages, marker=measure.PROCESS_MARKER):
    return "\n".join(marker + json.dumps(item) for item in stages)


@unittest.skipUnless(sys.platform == "linux", "Linux measurement driver")
class DriverTests(unittest.TestCase):
    def test_timeout_kills_and_reaps_workload(self):
        child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        with self.assertRaises(TimeoutError):
            measure.wait_for_workload(child, 0.05)
        self.assertLess(child.returncode, 0)
        with self.assertRaises(ChildProcessError):
            os.waitpid(child.pid, os.WNOHANG)

    def test_normal_exit_retains_child_usage(self):
        child = subprocess.Popen([sys.executable, "-c", "pass"])
        usage = measure.wait_for_workload(child, 5)
        self.assertEqual(child.returncode, 0)
        self.assertGreater(usage.ru_maxrss, 0)

    def test_output_aliases_are_rejected_before_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"preserve this input")
            symbolic = Path(directory) / "symbolic"
            symbolic.symlink_to(binary)
            hard = Path(directory) / "hard"
            os.link(binary, hard)
            for output in [binary, symbolic, hard]:
                with self.subTest(output=output), patch.object(measure, "run") as run:
                    self.invoke(binary, output, expected_code=2)
                    run.assert_not_called()
                    self.assertEqual(binary.read_bytes(), b"preserve this input")

    def test_failure_preserves_partial_samples_and_marks_report_incomplete(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"synthetic")
            output = Path(directory) / "result.json"
            sample = {"wall_s": 1, "cpu_s": 0.5, "maxrss_kib": 10, "stages": []}
            # Complete one base/head pair, then only the next scenario's base.
            with patch.object(measure, "run", side_effect=[sample, sample, sample, TimeoutError("stalled")]):
                self.invoke(binary, output, expected_code=1)
            report = json.loads(output.read_text())
            self.assertFalse(report["complete"])
            self.assertIn("TimeoutError", report["error"])
            self.assertEqual(len(report["runs"]), 3)
            self.assertEqual(set(report["summary"]["attached"]), {"base"})

    def test_process_markers_parse_in_stage_order(self):
        output = marker_text([stage(name, index) for index, name in enumerate(measure.ACCOUNT_STAGES)])
        stages = measure.collect_stages(output, "process", measure.ACCOUNT_STAGES)
        self.assertEqual([item["stage"] for item in stages], measure.ACCOUNT_STAGES)
        markers = measure.parse_markers(output)
        self.assertFalse(markers["memory"])
        self.assertFalse(markers["allocator"])
        reversed_output = marker_text([stage(name) for name in reversed(measure.ACCOUNT_STAGES)])
        with self.assertRaisesRegex(RuntimeError, "unexpected process checkpoints"):
            measure.collect_stages(reversed_output, "process", measure.ACCOUNT_STAGES)

    def test_timing_rejects_process_markers(self):
        output = marker_text([stage("startup")])
        for text in [output, "OXIDEZAP_ALLOCATOR counting\n" + output,
                     marker_text([stage("startup")], measure.MEMORY_MARKER)]:
            with self.subTest(text=text), self.assertRaisesRegex(RuntimeError, "Timing"):
                measure.collect_stages(text, "timing", measure.BRIDGE_STAGES)

    def test_idle_cpu_is_a_tick_delta(self):
        stages = [stage("startup", 0, 10, 20), stage("clients_ready", 50, 15, 25),
                  stage("idle_begin", 50, 15, 25), stage("idle_end", 550, 30, 65),
                  stage("shutdown_requested", 550, 30, 65), stage("released", 600, 40, 80)]
        with patch.object(measure.os, "sysconf", return_value=100):
            values = measure.derive(stages, measure.os.sysconf("SC_CLK_TCK"))
        self.assertAlmostEqual(values["idle_cpu_s"], (15 + 40) / 100)
        self.assertAlmostEqual(values["idle_wall_s"], 0.5)
        self.assertAlmostEqual(values["cold_wall_s"], 0.05)
        self.assertAlmostEqual(values["cold_cpu_s"], (5 + 5) / 100)
        self.assertAlmostEqual(values["shutdown_wall_s"], 0.05)
        self.assertEqual(values["peak_threads"], 2)
        self.assertEqual(values["peak_fds"], 4)
        self.assertEqual(values["peak_rss_kib"], 100)
        self.assertEqual(values["peak_pss_kib"], 50)

    def test_lifecycle_derivations_appear_in_summary(self):
        sample = {"wall_s": 1, "cpu_s": 0.5, "maxrss_kib": 10, "stages": []}
        sample.update({key: 2 for key in measure.DERIVED_KEYS})
        records = [{"scenario": "accounts_1", "variant": "base", "repeat": repeat, **sample}
                   for repeat in range(2)]
        data = measure.summarize(records)["accounts_1"]["base"]
        for key in measure.DERIVED_KEYS:
            self.assertEqual(data[key], {"median": 2, "min": 2, "max": 2})

    def test_partial_process_mode_failure_preserves_report(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"synthetic")
            output = Path(directory) / "result.json"
            sample = {"wall_s": 1, "cpu_s": 0.5, "maxrss_kib": 10, "stages": []}
            with patch.object(measure, "run", side_effect=[sample, sample, sample, TimeoutError("stalled")]):
                self.invoke(binary, output, expected_code=1, head=binary,
                            mode="process", idle_ms=250)
            report = json.loads(output.read_text())
            self.assertFalse(report["complete"])
            self.assertIn("TimeoutError", report["error"])
            self.assertEqual(report["mode"], "process")
            self.assertEqual(report["schema_version"], 2)
            self.assertEqual(report["idle_ms"], 250)
            self.assertGreater(report["clk_tck"], 0)
            self.assertEqual(report["expected_stages"]["accounts_1"], measure.ACCOUNT_STAGES)
            self.assertEqual(len(report["runs"]), 3)

    def test_unknown_stage_is_rejected(self):
        output = marker_text([stage("startup"), stage("not_a_stage")])
        with self.assertRaisesRegex(RuntimeError, "unexpected process checkpoints"):
            measure.collect_stages(output, "process", measure.ACCOUNT_STAGES)
        with self.assertRaisesRegex(RuntimeError, "Duplicate stage marker"):
            measure.derive([stage("startup"), stage("startup")], 100)

    def invoke(self, binary, output, expected_code, head=None, mode="timing", idle_ms=None):
        arguments = ["measure", "--base", str(binary), "--mode", mode, "--repeats", "1",
                     "--warmups", "0", "--output", str(output)]
        if idle_ms is not None:
            arguments += ["--idle-ms", str(idle_ms)]
        if head:
            arguments += ["--head", str(head)]
        with patch.object(sys, "argv", arguments), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as exited:
                measure.main()
        self.assertEqual(exited.exception.code, expected_code)


if __name__ == "__main__":
    unittest.main()

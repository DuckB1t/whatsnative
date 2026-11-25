#!/usr/bin/env python3
"""Run immutable release test executables in fresh, unprivileged Linux processes."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import math
import signal
import statistics
import subprocess
import tempfile
import time


SCHEMA_VERSION = 2
DEFAULT_IDLE_MS = 500
MAX_IDLE_MS = 60_000
MEMORY_MARKER = "OXIDEZAP_MEMORY "
PROCESS_MARKER = "OXIDEZAP_PROCESS "
ALLOCATOR_MARKER = "OXIDEZAP_ALLOCATOR counting"
REPORT_ENV = {"memory": "OXIDEZAP_MEMORY_REPORT", "process": "OXIDEZAP_PROCESS_REPORT"}
BASE_METRIC_KEYS = ["wall_s", "cpu_s", "maxrss_kib"]
DERIVED_KEYS = ["cold_wall_s", "cold_cpu_s", "idle_wall_s", "idle_cpu_s",
                "shutdown_wall_s", "peak_threads", "peak_fds", "peak_rss_kib",
                "peak_pss_kib"]
STAGE_METRIC_KEYS = ["live_bytes", "peak_bytes", "allocations", "requested_bytes"]


def wait_for_workload(process, timeout):
    def expired(_signal, _frame):
        raise TimeoutError(f"Workload exceeded {timeout:g} seconds")

    previous = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, timeout)
    try:
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        return usage
    except BaseException:
        # Prevent a stalled workload or an interrupted driver from leaving a child.
        signal.setitimer(signal.ITIMER_REAL, 0)
        if process.returncode is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
            _, status, _ = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        raise
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


BRIDGE_STAGES = ["startup", "initialized", "attach", "sync_queued", "drained",
                 "detached", "queried", "released"]
ACCOUNT_STAGES = ["startup", "registry_ready", "accounts_allocated", "stores_opened",
                  "clients_ready", "idle_begin", "idle_end", "shutdown_requested",
                  "clients_closed", "released"]
SCENARIOS = ["headless", "attached", "read", "groups",
             "accounts_1", "accounts_2", "accounts_4", "accounts_8"]
SCENARIO_STAGES = {
    "headless": BRIDGE_STAGES,
    "attached": BRIDGE_STAGES,
    "read": BRIDGE_STAGES,
    "groups": BRIDGE_STAGES,
    "accounts_1": ACCOUNT_STAGES,
    "accounts_2": ACCOUNT_STAGES,
    "accounts_4": ACCOUNT_STAGES,
    "accounts_8": ACCOUNT_STAGES,
}


def parse_markers(output):
    """Split captured stdout into machine-readable markers by kind."""
    markers = {"memory": [], "process": [],
               "allocator": ALLOCATOR_MARKER in output}
    for line in output.splitlines():
        if MEMORY_MARKER in line:
            markers["memory"].append(json.loads(line.split(MEMORY_MARKER, 1)[1]))
        if PROCESS_MARKER in line:
            markers["process"].append(json.loads(line.split(PROCESS_MARKER, 1)[1]))
    return markers


def validate_markers(markers, mode, expected):
    """Enforce the marker contract of one mode and return its stage list."""
    if mode == "memory":
        if markers["process"]:
            raise RuntimeError("Memory mode requires a binary without OXIDEZAP_PROCESS markers")
    elif mode == "process":
        if markers["memory"] or markers["allocator"]:
            raise RuntimeError("Process mode requires a binary built without memory-profiling")
    elif markers["memory"] or markers["process"] or markers["allocator"]:
        raise RuntimeError("Timing requires a binary built without profiling markers")
    stages = markers.get(mode, [])
    if mode in REPORT_ENV and [stage["stage"] for stage in stages] != expected:
        raise RuntimeError(f"Missing or unexpected {mode} checkpoints; use the correct test binary")
    return stages


def collect_stages(output, mode, expected):
    return validate_markers(parse_markers(output), mode, expected)


def _clock(stage, name):
    clock = stage.get("clock")
    if not isinstance(clock, dict):
        raise RuntimeError(f"Stage {name} has no clock")
    return clock


def _number(source, key, name):
    value = source.get(key)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise RuntimeError(f"Stage {name} has no numeric {key}")
    return value


def derive(stages, clk_tck):
    """Derive lifecycle windows and peak samples from stage markers.

    Cold is startup to clients_ready, idle is idle_begin to idle_end, and
    shutdown is shutdown_requested to released. CPU seconds come from tick
    deltas in the stage clocks; wall seconds come from their elapsed_ms.
    """
    if not stages:
        return {}
    if clk_tck <= 0:
        raise ValueError("clk_tck must be positive")
    indexed = {}
    for stage in stages:
        name = stage["stage"]
        if name in indexed:
            raise RuntimeError(f"Duplicate stage marker: {name}")
        indexed[name] = stage
    values = {}
    windows = (("cold", "startup", "clients_ready"),
               ("idle", "idle_begin", "idle_end"),
               ("shutdown", "shutdown_requested", "released"))
    for label, begin_name, end_name in windows:
        if begin_name not in indexed or end_name not in indexed:
            continue
        begin = _clock(indexed[begin_name], begin_name)
        end = _clock(indexed[end_name], end_name)
        values[f"{label}_wall_s"] = (
            _number(end, "elapsed_ms", end_name) - _number(begin, "elapsed_ms", begin_name)
        ) / 1000.0
        if label != "shutdown":
            ticks = sum(_number(end, key, end_name) - _number(begin, key, begin_name)
                        for key in ("cpu_utime_ticks", "cpu_stime_ticks"))
            values[f"{label}_cpu_s"] = ticks / clk_tck
    counts = [stage["process_counts"] for stage in stages
              if isinstance(stage.get("process_counts"), dict)]
    for key, label in (("threads", "peak_threads"), ("fds", "peak_fds")):
        numbers = [count[key] for count in counts
                   if isinstance(count.get(key), (int, float)) and not isinstance(count.get(key), bool)]
        if numbers:
            values[label] = max(numbers)
    kib = [stage["process_kib"] for stage in stages
           if isinstance(stage.get("process_kib"), dict)]
    for key, label in (("Rss", "peak_rss_kib"), ("Pss", "peak_pss_kib")):
        numbers = [sample[key] for sample in kib
                   if isinstance(sample.get(key), (int, float)) and not isinstance(sample.get(key), bool)]
        if numbers:
            values[label] = max(numbers)
    return values


def run(binary, scenario, mode, timeout, idle_ms, clk_tck):
    env = os.environ.copy()
    for name in REPORT_ENV.values():
        env.pop(name, None)
    env["OXIDEZAP_IDLE_MS"] = str(idle_ms)
    if mode in REPORT_ENV:
        env[REPORT_ENV[mode]] = "1"
    command = [str(binary), "--exact",
               f"session_bridge::memory_scenario::account_scale_memory_{scenario}",
               "--ignored", "--nocapture", "--test-threads=1"]
    with tempfile.TemporaryFile(mode="w+") as log:
        started = time.monotonic()
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        usage = wait_for_workload(process, timeout)
        elapsed = time.monotonic() - started
        log.seek(0)
        output = log.read()
    if process.returncode != 0:
        raise RuntimeError(f"{command} failed:\n{output}")
    if "1 passed; 0 failed" not in output:
        raise RuntimeError(f"Workload was not executed exactly once:\n{output}")
    stages = collect_stages(output, mode, SCENARIO_STAGES[scenario])
    sample = {"wall_s": elapsed, "cpu_s": usage.ru_utime + usage.ru_stime,
              "user_s": usage.ru_utime, "system_s": usage.ru_stime,
              "maxrss_kib": usage.ru_maxrss, "stages": stages}
    sample.update(derive(stages, clk_tck))
    return sample


def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


def summarize(records):
    result = {}
    for scenario in sorted({r["scenario"] for r in records}):
        result[scenario] = {}
        for variant in sorted({r["variant"] for r in records if r["scenario"] == scenario}):
            selected = [r for r in records if r["scenario"] == scenario and r["variant"] == variant]
            data = {key: distribution([r[key] for r in selected]) for key in BASE_METRIC_KEYS}
            for key in DERIVED_KEYS:
                if all(key in r for r in selected):
                    data[key] = distribution([r[key] for r in selected])
            data["stages"] = {}
            if selected[0]["stages"]:
                for index, first in enumerate(selected[0]["stages"]):
                    samples = [r["stages"][index] for r in selected]
                    stage = {key: distribution([s[key] for s in samples])
                             for key in STAGE_METRIC_KEYS
                             if isinstance(first.get(key), (int, float))}
                    if isinstance(first.get("clock"), dict):
                        stage["clock"] = {key: distribution([s["clock"][key] for s in samples])
                                          for key in first["clock"]}
                    if isinstance(first.get("process_kib"), dict):
                        stage["process_kib"] = {
                            key: distribution([s["process_kib"][key] for s in samples])
                            for key in first["process_kib"]
                        }
                    if isinstance(first.get("process_counts"), dict):
                        stage["process_counts"] = {
                            key: distribution([s["process_counts"][key] for s in samples])
                            for key in first["process_counts"]
                        }
                    data["stages"][first["stage"]] = stage
            result[scenario][variant] = data
    return result


def binary_info(path):
    """Identify one immutable binary, carrying build-feature metadata if present."""
    info = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    sidecar = path.with_suffix(path.suffix + ".build.json")
    if sidecar.is_file():
        try:
            build = json.loads(sidecar.read_text())
        except (OSError, ValueError):
            build = {}
        if isinstance(build, dict):
            for key in ("feature", "features", "build_mode", "instrumented"):
                if key in build:
                    info[key] = build[key]
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, type=Path)
    parser.add_argument("--head", type=Path)
    parser.add_argument("--mode", choices=["memory", "timing", "process"], required=True)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--idle-ms", type=int, default=DEFAULT_IDLE_MS,
                        help="Fixed no-event window in the process workload")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=120, help="Seconds per workload process")
    args = parser.parse_args()
    if platform.system() != "Linux" or args.repeats < 1 or args.warmups < 0:
        parser.error("Requires Linux, positive repeats and nonnegative warmups")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    if args.idle_ms < 0 or args.idle_ms > MAX_IDLE_MS:
        parser.error(f"--idle-ms must be between 0 and {MAX_IDLE_MS}")
    binaries = {"base": args.base.resolve(strict=True)}
    if args.head:
        binaries["head"] = args.head.resolve(strict=True)
    output = args.output.resolve()
    if any(output == binary or (output.exists() and output.samefile(binary))
           for binary in binaries.values()):
        parser.error("--output must not refer to an input binary")
    clk_tck = int(os.sysconf("SC_CLK_TCK"))
    scenarios = SCENARIOS
    records = []
    failure = None
    try:
        for scenario in scenarios:
            for _ in range(args.warmups):
                for binary in binaries.values():
                    run(binary, scenario, args.mode, args.timeout, args.idle_ms, clk_tck)
            for repeat in range(args.repeats):
                order = list(binaries)
                if repeat % 2:
                    order.reverse()
                for variant in order:
                    sample = run(binaries[variant], scenario, args.mode, args.timeout,
                                 args.idle_ms, clk_tck)
                    records.append({"scenario": scenario, "variant": variant, "repeat": repeat, **sample})
    except (Exception, KeyboardInterrupt) as error:
        failure = f"{type(error).__name__}: {error}"
    model = next((line.split(":", 1)[1].strip() for line in
                  Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), "unknown")
    result = {"schema_version": SCHEMA_VERSION, "mode": args.mode,
              "repeats": args.repeats, "warmups": args.warmups,
              "timeout_s": args.timeout, "clk_tck": clk_tck, "idle_ms": args.idle_ms,
              "expected_stages": {name: list(stages) for name, stages in SCENARIO_STAGES.items()},
              "complete": failure is None, "error": failure,
              "runner": {"machine": platform.machine(), "kernel": platform.release(),
                         "cpu_model": model, "affinity": sorted(os.sched_getaffinity(0))},
              "binaries": {name: binary_info(path) for name, path in binaries.items()},
              "summary": summarize(records), "runs": records}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Wrote {len(records)} isolated runs to {args.output}")
    if failure is not None:
        parser.exit(1, failure + "\n")


if __name__ == "__main__":
    main()

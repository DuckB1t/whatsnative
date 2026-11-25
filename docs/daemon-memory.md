# Daemon memory measurements

The harness uses fresh Linux processes, the existing release profile, `/proc`
and `wait4`. No root, eBPF, CodSpeed, credentials or application dependencies are
needed. Build immutable binaries on the baseline revision, then the candidate,
using the same toolchain, runner and profile:

```sh
python3 scripts/build-memory-workload.py --instrument /tmp/base-memory
python3 scripts/build-memory-workload.py --process /tmp/base-process
python3 scripts/build-memory-workload.py /tmp/base-timing
# Switch to the candidate revision, keeping the same harness.
python3 scripts/build-memory-workload.py --instrument /tmp/head-memory
python3 scripts/build-memory-workload.py --process /tmp/head-process
python3 scripts/build-memory-workload.py /tmp/head-timing
python3 scripts/measure-daemon-memory.py --mode memory --base /tmp/base-memory --head /tmp/head-memory --output /tmp/memory.json
python3 scripts/measure-daemon-memory.py --mode process --base /tmp/base-process --head /tmp/head-process --idle-ms 500 --output /tmp/process.json
python3 scripts/measure-daemon-memory.py --mode timing --base /tmp/base-timing --head /tmp/head-timing --output /tmp/timing.json
```

The driver alternates base/head order, excludes one warmup per scenario and
reports seven isolated samples with median/min/max and raw observations. Each
binary has a recorded SHA-256; build metadata records the compiler, revision and
profiling feature. The OS file cache is warm, not forcibly cleared. Do not
compare different hosts. Each workload has a configurable `--timeout` (120
seconds by default); failed runs exit nonzero and preserve completed samples in
an explicitly incomplete report. Output paths that alias an input binary are
rejected before execution.

The three modes are not interchangeable. `memory` uses the counting allocator
and emits `OXIDEZAP_MEMORY` markers. `process` uses process probes only and
emits `OXIDEZAP_PROCESS` markers, with no counting allocator and no requested
heap bytes. `timing` uses a plain test binary and rejects every marker. A mode
that sees another mode's marker fails instead of quietly mixing measurements.
The report records `schema_version`, `clk_tck`, `idle_ms`, the expected stage
list per scenario and the build feature of each binary.

The bridge workloads replay 75,703 fixed-size synthetic text messages over 1,032
chats, after a 100-chat attach with eight read rows per chat. Cases cover unread
traffic with/without a subscriber, already-read messages, and groups with eight
alternating authors. The attached subscriber deliberately lags until the publisher
drains. Assertions cover duplicate delivery, badges, receipt queries and the
publication tail. `account_scale_paging` separately checks 1,653 contacts and
75,703 SQLite rows; it is not part of these bridge memory numbers.

The account-scale workloads create one real throwaway SQLite file, allocate 1,
2, 4 or 8 accounts through one `StoreRegistry`, prepare the shared schema once,
construct that many clients, and never start or pair them. After a fixed quiet
window they close the clients and drop every handle. Stages are `startup`,
`registry_ready`, `accounts_allocated`, `stores_opened`, `clients_ready`,
`idle_begin`, `idle_end`, `shutdown_requested`, `clients_closed` and
`released`. Cold construction is `startup` to `clients_ready`. Idle is
`idle_begin` to `idle_end`. Shutdown here is `shutdown_requested` to
`released`. Wall seconds come from the stage clocks. CPU seconds for the cold
and idle windows come from `/proc/self/stat` tick deltas divided by
`SC_CLK_TCK`; shutdown CPU is not derived because close may legitimately wait.
The idle window is set once by `--idle-ms` and the driver passes the same
value to the workload. These are construction and teardown measurements, not connected
account benchmarks.

The opt-in `memory-profiling` feature installs a counting allocator in native
test binaries only. It measures requested Rust heap bytes, live and peak, and
allocation traffic across all threads. It excludes allocator metadata, C
allocations and alignment slack. Checkpoints separate initialization, attach,
queued sync, drained publication, detach, receipt queries and released state.
Sampling and formatting allocate small amounts themselves. `realloc` traffic
counts its requested new size, not the unknown amount copied by the allocator.

RSS/PSS, anonymous/file/shared resident pages and the kernel's RSS high-water mark
are reported separately. A fall in live heap without an equal RSS fall can include
allocator retention, stacks and untracked allocations; it does not establish an
exact allocator-retained byte count. File-backed pages are not live Rust heap.
Timing binaries have no counting allocator or memory probes; CPU/wall values
include process/test startup and are reported as distributions, not universal
latency guarantees.

The bridge workloads isolate bridge bookkeeping and publication. The account
workloads add construction and teardown of never-started clients. Neither
represents a whole authenticated daemon, cryptographic state, history sync,
reconnect, media or calls. Whole-daemon startup/idle claims require a separate
scenario. Production binaries keep their existing allocator even when a
profiling feature is enabled.

## Read-tracker compaction (2026-10-29)

Compared harness baseline `1ccaaf3` with production change `9086fcc`, using
Rust 1.99.1, the repository release profile, Linux 6.18.55, AMD EPYC 9V74 and
the same five-CPU affinity. Immutable IDs and sender strings now use `Box<str>`
instead of retaining unused `String` capacity fields. Cache limits, receipt
semantics and public types are unchanged. Draining receipts transfers the
buffers back to strings without copying. No runtime dependency was added.

Seven paired samples measured heap; fifteen paired uninstrumented samples
measured peak RSS and CPU/wall time. Each excluded one warmup per variant and
scenario. Equal-length executable paths kept startup allocations identical.

| Scenario | Live heap after drain, base → head (MiB) | Peak RSS, base → head (KiB; median [min–max]) |
| --- | --- | --- |
| headless | 17.30 → 14.18 | 32340 [32156–32532] → 28828 [28792–29064] |
| attached | 17.80 → 14.68 | 35204 [34916–35328] → 31468 [29880–31852] |
| groups | 17.28 → 14.16 | 33068 [31504–33132] → 29500 [29260–29636] |
| read | 6.92 → 5.82 | 18844 [17304–19200] → 17552 [16016–17864] |

Unread scenarios save exactly 3,269,376 requested live bytes (3.12 MiB) in
every sample: about 18% after drain and 24% after receipt queries. Already-read
traffic saves 1,155,840 bytes (16%). Startup and released live bytes match the
baseline. Anonymous PSS after drain falls by approximately 3.1 MiB for unread
traffic and 1.1 MiB for read traffic; resident pages can remain after objects
are freed, so these are not exact allocator-retention measurements.

| Scenario | CPU seconds, base → head (median [min–max]) | Wall seconds, base → head (median [min–max]) |
| --- | --- | --- |
| headless | 0.365 [0.347–0.453] → 0.358 [0.334–0.403] | 0.366 [0.347–0.451] → 0.358 [0.335–0.403] |
| attached | 0.929 [0.881–1.266] → 0.918 [0.875–1.237] | 0.751 [0.712–0.987] → 0.744 [0.704–0.963] |
| groups | 0.394 [0.367–0.454] → 0.386 [0.354–0.537] | 0.394 [0.367–0.454] → 0.386 [0.354–0.537] |
| read | 0.313 [0.292–0.347] → 0.308 [0.293–0.339] | 0.313 [0.292–0.347] → 0.308 [0.293–0.339] |

The distributions overlap. A preceding independent paired run also reduced
RSS, but CPU/wall medians varied slightly in both directions. This establishes
a memory reduction in these workloads, not a universal latency improvement or
a measured reduction for a complete connected account.

## Wave 4 synthetic process characterization (2026-11-25)

The item 10 working tree was measured before its commit with Rust 1.100.0, the
repository release profile and one immutable `process-profiling` binary. Two
copies of that same binary were used as base and head, with the same SHA-256
`c12f7fc6126f724da4807e12d15c3afd69e88db18d15317d3fe3068139fd4def`, so the
driver's alternating order and warmup were exercised without pretending there
was a before-and-after change. The runner was Linux 7.3.5-2-cachyos on x86_64,
an Intel i7-8665U with eight CPUs in the affinity mask. One warmup and seven
samples per scenario and variant produced 112 runs with a 500 ms idle window
and a 120 s per process timeout.

| Synthetic accounts | Cold wall s, median [min-max] | Cold CPU s, median [min-max] | Idle CPU s | Shutdown wall s, median [min-max] | Peak threads | Peak fds | Stage peak RSS KiB, median [min-max] | Stage peak PSS KiB, median [min-max] | wait4 max RSS KiB, median [min-max] |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 0.034 [0.032-0.039] | 0.03 [0.02-0.03] | 0.0 | 0.001 [0.001-0.002] | 13 | 19 | 17192 [17104-17472] | 12952 [12822-13197] | 26972 [26844-27100] |
| 2 | 0.036 [0.034-0.037] | 0.03 [0.02-0.03] | 0.0 | 0.002 [0.001-0.003] | 21 | 23 | 17476 [17204-17640] | 13207 [12936-13350] | 27356 [27228-27484] |
| 4 | 0.037 [0.035-0.043] | 0.03 [0.03-0.04] | 0.0 | 0.002 [0.002-0.003] | 37 | 31 | 17400 [17212-17704] | 13132 [12937-13430] | 27740 [27484-27740] |
| 8 | 0.033 [0.033-0.036] | 0.03 [0.02-0.03] | 0.0 | 0.003 [0.002-0.003] | 69 | 47 | 17744 [17416-17936] | 13667 [13337-13864] | 27996 [27868-28252] |

The paired copy produced overlapping distributions in every row; the table
uses the base copy's medians, and the head copy is in the report. Thread and fd
counts were identical across all samples at each scale: the first client sits
at 13 threads and 19 fds, and each additional never-started client adds eight
threads and four fds. That confirms the concern that N clients are N private
runtimes in this harness, instead of one shared daemon runtime. Cold wall
medians overlap and do not increase monotonically, so this short workload is
not a latency scaling claim.

Idle wall is 0.501 s by construction. Idle CPU observed by 100 Hz `/proc`
ticks was zero in every sample, which means below one tick of user plus system
time, not a proof that no code ran. Cold CPU has the same tick resolution and
cannot support a per-account CPU claim. Stage RSS/PSS samples and `wait4`
`maxrss` are different populations; the stage values are checkpoints, and
`maxrss` is the whole-process high-water mark. Neither gives per-account RSS,
PSS or fd attribution, and no allocator retention is inferred from them.

No browser wasm tests, history sync, reconnect, authenticated account, call,
message, media path or real PII is involved. A connected-account result needs
an authorized account and a separate scenario. Anyone calling this a
connected-account benchmark is full of shit.

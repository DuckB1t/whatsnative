//! Process clocks, process probes and optional requested-heap counters.
//! Opt-in native test instrumentation only: no production binary links this
//! module, and neither feature replaces a production allocator.

// Machine-readable output is the test harness's measurement interface.
#![allow(clippy::print_stdout)]

use wacore::time::Instant;

/// Every marker's `elapsed_ms` is measured from this one `begin()` origin.
static BEGAN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

#[cfg(feature = "memory-profiling")]
mod counting {
    use portable_atomic::{AtomicUsize, Ordering};
    use std::alloc::{GlobalAlloc, Layout, System};

    struct Counting;

    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);
    static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
    static REQUESTED: AtomicUsize = AtomicUsize::new(0);

    fn added(size: usize) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        REQUESTED.fetch_add(size, Ordering::Relaxed);
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(live, Ordering::Relaxed);
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            // SAFETY: pass the allocation contract through to System unchanged.
            let ptr = unsafe { System.alloc(layout) };
            if !ptr.is_null() {
                added(layout.size());
            }
            ptr
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            // SAFETY: pass the allocation contract through to System unchanged.
            let ptr = unsafe { System.alloc_zeroed(layout) };
            if !ptr.is_null() {
                added(layout.size());
            }
            ptr
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            // SAFETY: the caller provides the original pointer and layout.
            unsafe { System.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            // SAFETY: the caller provides a live pointer/layout and a valid size.
            let next = unsafe { System.realloc(ptr, layout, size) };
            if !next.is_null() {
                ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                REQUESTED.fetch_add(size, Ordering::Relaxed);
                let live = if size >= layout.size() {
                    LIVE.fetch_add(size - layout.size(), Ordering::Relaxed) + size - layout.size()
                } else {
                    LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed) - (layout.size() - size)
                };
                PEAK.fetch_max(live, Ordering::Relaxed);
            }
            next
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    pub(super) struct Samples {
        pub(super) live: usize,
        pub(super) peak: usize,
        pub(super) allocations: usize,
        pub(super) requested: usize,
    }

    pub(super) fn reset_peak() {
        PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    pub(super) fn samples() -> Samples {
        Samples {
            live: LIVE.load(Ordering::Relaxed),
            peak: PEAK.load(Ordering::Relaxed),
            allocations: ALLOCATIONS.load(Ordering::Relaxed),
            requested: REQUESTED.load(Ordering::Relaxed),
        }
    }
}

mod clock {
    /// Read `utime` and `stime` as kernel ticks from `/proc/self/stat`.
    /// The command name is parenthesized and can itself contain `)`, so the
    /// line is split at the final `)`. The tail starts at field 3 (`state`);
    /// `utime` and `stime` are fields 14 and 15, eleven tokens after `state`.
    pub(super) fn cpu_ticks() -> (u64, u64) {
        #[cfg(target_os = "linux")]
        {
            let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
                return (0, 0);
            };
            let Some((_, tail)) = stat.rsplit_once(')') else {
                return (0, 0);
            };
            let mut fields = tail.split_whitespace().skip(11);
            let utime = fields
                .next()
                .and_then(|field| field.parse().ok())
                .unwrap_or(0);
            let stime = fields
                .next()
                .and_then(|field| field.parse().ok())
                .unwrap_or(0);
            (utime, stime)
        }
        #[cfg(not(target_os = "linux"))]
        {
            (0, 0)
        }
    }
}

fn clock() -> serde_json::Value {
    let elapsed_ms = BEGAN
        .get()
        .map(|began| began.elapsed().as_millis() as u64)
        .unwrap_or(0);
    let (utime, stime) = clock::cpu_ticks();
    serde_json::json!({
        "elapsed_ms": elapsed_ms,
        "cpu_utime_ticks": utime,
        "cpu_stime_ticks": stime,
    })
}

pub(crate) fn begin() {
    #[cfg(feature = "memory-profiling")]
    counting::reset_peak();
    BEGAN.get_or_init(Instant::now);
    #[cfg(feature = "memory-profiling")]
    println!("OXIDEZAP_ALLOCATOR counting");
}

pub(crate) fn checkpoint(stage: &str) {
    #[cfg(feature = "memory-profiling")]
    let report_memory = std::env::var_os("OXIDEZAP_MEMORY_REPORT").is_some();
    #[cfg(not(feature = "memory-profiling"))]
    let report_memory = false;
    // A process-only build emits no allocator or requested-heap markers. When
    // memory-profiling is on, it owns the marker even though it also enables
    // the process probes.
    let report_process = cfg!(all(
        feature = "process-profiling",
        not(feature = "memory-profiling")
    )) && std::env::var_os("OXIDEZAP_PROCESS_REPORT").is_some();
    if !report_memory && !report_process {
        return;
    }
    // Read counters before the sampling/formatting allocations. Samples
    // themselves are counted in subsequent checkpoints on both revisions.
    #[cfg(feature = "memory-profiling")]
    let samples = counting::samples();
    let snapshot = process::snapshot();
    let clock = clock();
    #[cfg(feature = "memory-profiling")]
    if report_memory {
        println!(
            "OXIDEZAP_MEMORY {}",
            serde_json::json!({
                "stage": stage, "clock": clock,
                "live_bytes": samples.live, "peak_bytes": samples.peak,
                "allocations": samples.allocations, "requested_bytes": samples.requested,
                "process_kib": snapshot.kib, "process_counts": snapshot.counts,
            })
        );
    }
    #[cfg(all(feature = "process-profiling", not(feature = "memory-profiling")))]
    if report_process {
        println!(
            "OXIDEZAP_PROCESS {}",
            serde_json::json!({
                "stage": stage, "clock": clock,
                "process_kib": snapshot.kib, "process_counts": snapshot.counts,
            })
        );
    }
}

#[cfg(target_os = "linux")]
mod process {
    /// Count entries the kernel exposes in one proc directory. The directory
    /// descriptor used for the listing is itself counted as an open fd, so the
    /// reported fd count includes the sampler's own `readdir` descriptor.
    fn count_entries(path: &str) -> u64 {
        std::fs::read_dir(path)
            .map(|entries| entries.filter_map(Result::ok).count() as u64)
            .unwrap_or(0)
    }

    pub(super) struct Snapshot {
        pub(super) kib: serde_json::Value,
        pub(super) counts: serde_json::Value,
    }

    pub(super) fn snapshot() -> Snapshot {
        let mut fields = serde_json::Map::new();
        for path in ["/proc/self/smaps_rollup", "/proc/self/status"] {
            if let Ok(contents) = std::fs::read_to_string(path) {
                for line in contents.lines() {
                    let Some((name, value)) = line.split_once(':') else {
                        continue;
                    };
                    if matches!(
                        name,
                        "Rss"
                            | "Pss"
                            | "Pss_Anon"
                            | "Pss_File"
                            | "Pss_Shmem"
                            | "Anonymous"
                            | "Private_Clean"
                            | "Private_Dirty"
                            | "Shared_Clean"
                            | "Shared_Dirty"
                            | "Swap"
                            | "VmHWM"
                            | "RssAnon"
                            | "RssFile"
                            | "RssShmem"
                    ) && let Some(number) = value
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        fields.insert(name.into(), number.into());
                    }
                }
            }
        }
        Snapshot {
            kib: serde_json::Value::Object(fields),
            counts: serde_json::json!({
                "threads": count_entries("/proc/self/task"),
                "fds": count_entries("/proc/self/fd"),
            }),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod process {
    pub(super) struct Snapshot {
        pub(super) kib: serde_json::Value,
        pub(super) counts: serde_json::Value,
    }

    pub(super) fn snapshot() -> Snapshot {
        Snapshot {
            kib: serde_json::Value::Null,
            counts: serde_json::Value::Null,
        }
    }
}

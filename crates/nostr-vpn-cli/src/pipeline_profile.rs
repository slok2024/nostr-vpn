use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

const N_STAGES: usize = 4;
const HIST_BUCKETS: usize = 48;

#[derive(Copy, Clone)]
#[repr(usize)]
pub(crate) enum Stage {
    TunRead = 0,
    TunToMeshQueueWait = 1,
    MeshSend = 2,
    TunWrite = 3,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::TunRead => "nvpn_tun_read",
            Stage::TunToMeshQueueWait => "nvpn_tun_to_mesh_queue_wait",
            Stage::MeshSend => "nvpn_mesh_send",
            Stage::TunWrite => "nvpn_tun_write",
        }
    }
}

fn stage_from_index(idx: usize) -> Stage {
    match idx {
        0 => Stage::TunRead,
        1 => Stage::TunToMeshQueueWait,
        2 => Stage::MeshSend,
        3 => Stage::TunWrite,
        _ => unreachable!(),
    }
}

static TOTAL_NS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static COUNT: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static MAX_NS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
static HIST: [AtomicU64; N_STAGES * HIST_BUCKETS] =
    [const { AtomicU64::new(0) }; N_STAGES * HIST_BUCKETS];

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        ["NVPN_PIPELINE_TRACE", "FIPS_PIPELINE_TRACE"]
            .into_iter()
            .any(|key| {
                std::env::var(key)
                    .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
            })
    })
}

#[inline]
pub(crate) fn stamp() -> Option<Instant> {
    enabled().then(Instant::now)
}

#[inline]
pub(crate) fn record_since(stage: Stage, start: Option<Instant>) {
    if let Some(start) = start {
        record(stage, start.elapsed().as_nanos() as u64);
    }
}

pub(crate) fn record(stage: Stage, elapsed_ns: u64) {
    if !enabled() {
        return;
    }
    let idx = stage as usize;
    let elapsed_ns = elapsed_ns.max(1);
    TOTAL_NS[idx].fetch_add(elapsed_ns, Relaxed);
    COUNT[idx].fetch_add(1, Relaxed);
    MAX_NS[idx].fetch_max(elapsed_ns, Relaxed);
    HIST[(idx * HIST_BUCKETS) + bucket_for_ns(elapsed_ns)].fetch_add(1, Relaxed);
}

pub(crate) struct Timer {
    stage: Stage,
    start: Option<Instant>,
}

impl Timer {
    #[inline]
    pub(crate) fn start(stage: Stage) -> Self {
        Self {
            stage,
            start: stamp(),
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        record_since(self.stage, self.start);
    }
}

pub(crate) fn maybe_spawn_reporter() {
    if !enabled() {
        return;
    }
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let interval = std::env::var("NVPN_PIPELINE_INTERVAL_SECS")
        .ok()
        .or_else(|| std::env::var("FIPS_PERF_INTERVAL_SECS").ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5)
        .max(1);
    tokio::spawn(async move {
        let mut prev_total = [0u64; N_STAGES];
        let mut prev_count = [0u64; N_STAGES];
        let mut prev_hist = [0u64; N_STAGES * HIST_BUCKETS];
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            let mut line = format!("[nvpn-pipe {}s]", interval);
            for i in 0..N_STAGES {
                let total = TOTAL_NS[i].load(Relaxed);
                let count = COUNT[i].load(Relaxed);
                let dt = total.saturating_sub(prev_total[i]);
                let dc = count.saturating_sub(prev_count[i]);
                prev_total[i] = total;
                prev_count[i] = count;

                let base = i * HIST_BUCKETS;
                let mut hist_delta = [0u64; HIST_BUCKETS];
                for (bucket, slot) in hist_delta.iter_mut().enumerate() {
                    let idx = base + bucket;
                    let current = HIST[idx].load(Relaxed);
                    *slot = current.saturating_sub(prev_hist[idx]);
                    prev_hist[idx] = current;
                }
                if dc == 0 {
                    continue;
                }

                let stage = stage_from_index(i);
                let avg_ns = dt / dc;
                let pps = dc / interval;
                let p50 = percentile_ns(&hist_delta, dc, 50);
                let p95 = percentile_ns(&hist_delta, dc, 95);
                let p99 = percentile_ns(&hist_delta, dc, 99);
                let approx_max = interval_max_ns(&hist_delta);
                let lifetime_max = MAX_NS[i].load(Relaxed);
                line.push_str(&format!(
                    " {}={}/s avg={} p50<={} p95<={} p99<={} max<={} allmax={}",
                    stage.name(),
                    pps,
                    fmt_ns(avg_ns),
                    fmt_ns(p50),
                    fmt_ns(p95),
                    fmt_ns(p99),
                    fmt_ns(approx_max),
                    fmt_ns(lifetime_max),
                ));
            }
            eprintln!("{line}");
        }
    });
}

fn bucket_for_ns(ns: u64) -> usize {
    if ns <= 1 {
        return 0;
    }
    ((u64::BITS - (ns - 1).leading_zeros()) as usize).min(HIST_BUCKETS - 1)
}

fn bucket_upper_ns(bucket: usize) -> u64 {
    if bucket == 0 {
        1
    } else if bucket >= 63 {
        u64::MAX
    } else {
        1u64 << bucket
    }
}

fn percentile_ns(hist_delta: &[u64; HIST_BUCKETS], total: u64, pct: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    let target = total.saturating_mul(pct).saturating_add(99) / 100;
    let mut seen = 0u64;
    for (idx, count) in hist_delta.iter().enumerate() {
        seen = seen.saturating_add(*count);
        if seen >= target {
            return bucket_upper_ns(idx);
        }
    }
    bucket_upper_ns(HIST_BUCKETS - 1)
}

fn interval_max_ns(hist_delta: &[u64; HIST_BUCKETS]) -> u64 {
    for idx in (0..HIST_BUCKETS).rev() {
        if hist_delta[idx] != 0 {
            return bucket_upper_ns(idx);
        }
    }
    0
}

fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.1}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.1}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.1}us", ns as f64 / 1_000.0)
    } else {
        format!("{ns}ns")
    }
}

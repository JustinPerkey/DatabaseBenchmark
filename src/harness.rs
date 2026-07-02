use anyhow::Result;
use std::time::Instant;

/// Per-operation iteration counts, tuned so the whole run finishes in a couple
/// of minutes while still giving stable medians.
#[derive(Clone, Copy)]
pub struct BenchConfig {
    pub warmup: u32,
    pub iters: u32,
    pub bulk_iters: u32,
    pub read_iters: u32,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            warmup: 30,
            iters: 400,
            bulk_iters: 40,
            read_iters: 200,
        }
    }
}

impl BenchConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("BENCH_ITERS") {
            if let Ok(n) = v.parse() {
                cfg.iters = n;
            }
        }
        if let Ok(v) = std::env::var("BENCH_QUICK") {
            if v == "1" {
                cfg = Self {
                    warmup: 5,
                    iters: 40,
                    bulk_iters: 5,
                    read_iters: 20,
                };
            }
        }
        cfg
    }
}

/// Timing statistics for one operation on one suite, in microseconds.
pub struct OpStats {
    pub mean_us: f64,
    pub median_us: f64,
    pub p95_us: f64,
}

impl OpStats {
    pub fn ops_per_sec(&self) -> f64 {
        if self.median_us > 0.0 {
            1_000_000.0 / self.median_us
        } else {
            f64::INFINITY
        }
    }
}

/// Run `f` for `warmup` untimed iterations, then `iters` timed iterations,
/// and return the timing distribution. The iteration index is passed through
/// so operations can target distinct rows.
pub fn run_op(warmup: u32, iters: u32, mut f: impl FnMut(u32) -> Result<()>) -> Result<OpStats> {
    for i in 0..warmup {
        f(i)?;
    }
    let mut samples = Vec::with_capacity(iters as usize);
    for i in 0..iters {
        let start = Instant::now();
        f(warmup + i)?;
        samples.push(start.elapsed().as_secs_f64() * 1_000_000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    let median = percentile(&samples, 50.0);
    let p95 = percentile(&samples, 95.0);
    Ok(OpStats {
        mean_us: mean,
        median_us: median,
        p95_us: p95,
    })
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo as f64)
    }
}

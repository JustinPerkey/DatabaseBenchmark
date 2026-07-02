mod harness;
mod suites;

use anyhow::Result;
use harness::{run_op, BenchConfig, OpStats};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use suites::Suite;

const OPS: [&str; 7] = [
    "insert_one",
    "insert_bulk_100",
    "fetch_by_id",
    "fetch_page_50",
    "join_top_50",
    "update_one",
    "delete_one",
];
/// Indices into OPS of the read operations (the only ones read-only suites run).
const READ_OPS: [usize; 3] = [2, 3, 4];

struct SuiteResult {
    name: &'static str,
    /// One entry per OPS; None when the suite is read-only and the op writes.
    stats: Vec<Option<OpStats>>,
}

fn main() -> Result<()> {
    let cfg = BenchConfig::from_env();
    let tmp = std::env::temp_dir().join("rust-db-benchmark");
    std::fs::create_dir_all(&tmp)?;

    let pg_available = postgres_reachable();
    if !pg_available {
        eprintln!(
            "warning: PostgreSQL is not reachable at 127.0.0.1:5432 — running SQLite suites only.\n\
             Start one with: docker run -d -p 5432:5432 -e POSTGRES_USER=bench \\\n\
               -e POSTGRES_PASSWORD=bench -e POSTGRES_DB=bench postgres:16"
        );
    }

    let sqlite_db = |name: &str| {
        let path = tmp.join(format!("{name}.db"));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(tmp.join(format!("{name}.db{suffix}")));
        }
        path
    };

    let mut all_suites: Vec<Box<dyn Suite>> = vec![
        Box::new(suites::rusqlite_sqlite::RusqliteSqlite::new(sqlite_db(
            "rusqlite",
        ))?),
        Box::new(
            suites::rusqlite_sqlite_readonly::RusqliteSqliteReadonly::new(sqlite_db(
                "rusqlite_ro",
            ))?,
        ),
        Box::new(suites::sqlx_sqlite::SqlxSqlite::new(sqlite_db("sqlx"))?),
        Box::new(suites::diesel_sqlite::DieselSqlite::new(sqlite_db(
            "diesel",
        ))?),
        Box::new(suites::seaorm_sqlite::SeaOrmSqlite::new(sqlite_db(
            "seaorm",
        ))?),
    ];
    if pg_available {
        all_suites.push(Box::new(suites::tokio_postgres_pg::TokioPostgres::new()?));
        all_suites.push(Box::new(suites::sqlx_postgres::SqlxPostgres::new()?));
        all_suites.push(Box::new(suites::diesel_postgres::DieselPostgres::new()?));
        all_suites.push(Box::new(suites::seaorm_postgres::SeaOrmPostgres::new()?));
    }

    let delete_rows = cfg.warmup + cfg.iters;
    let mut results = Vec::new();
    for suite in &mut all_suites {
        eprintln!("==> {}", suite.name());
        suite.setup(delete_rows)?;
        let stats = run_suite(suite.as_mut(), &cfg)?;
        suite.teardown()?;
        results.push(SuiteResult {
            name: suite.name(),
            stats,
        });
    }
    drop(all_suites);

    let report = render_report(&results, &cfg);
    print!("{report}");
    std::fs::write("RESULTS.md", &report)?;
    eprintln!("\nwrote RESULTS.md");
    Ok(())
}

fn run_suite(suite: &mut dyn Suite, cfg: &BenchConfig) -> Result<Vec<Option<OpStats>>> {
    let w = cfg.warmup;
    let writes = !suite.read_only();
    // Order matches OPS.
    let stats = vec![
        if writes {
            Some(run_op(w, cfg.iters, |i| suite.insert_one(i))?)
        } else {
            None
        },
        if writes {
            Some(run_op(w.min(5), cfg.bulk_iters, |i| suite.insert_bulk(i))?)
        } else {
            None
        },
        Some(run_op(w, cfg.iters, |i| suite.fetch_by_id(i))?),
        Some(run_op(w, cfg.read_iters, |_| suite.fetch_page())?),
        Some(run_op(w, cfg.read_iters, |_| suite.join_query())?),
        if writes {
            Some(run_op(w, cfg.iters, |i| suite.update_one(i))?)
        } else {
            None
        },
        if writes {
            Some(run_op(w, cfg.iters, |i| suite.delete_one(i))?)
        } else {
            None
        },
    ];
    Ok(stats)
}

fn postgres_reachable() -> bool {
    let addr: SocketAddr = "127.0.0.1:5432".parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

fn fmt_us(us: f64) -> String {
    if us >= 10_000.0 {
        format!("{:.2} ms", us / 1000.0)
    } else {
        format!("{us:.1} µs")
    }
}

/// Rank a group of suites by the geometric mean of each suite's per-op
/// slowdown vs. the group's fastest, over the given OPS indices. Every suite
/// in `group` must have stats for every index requested.
fn ranking_table(group: &[&SuiteResult], op_indices: &[usize]) -> String {
    let mut out = String::from("| Rank | Suite | Relative latency |\n|---|---|---:|\n");
    let mut scored: Vec<(&str, f64)> = group
        .iter()
        .map(|r| {
            let mut log_sum = 0.0;
            for &op_idx in op_indices {
                let best = group
                    .iter()
                    .filter_map(|g| g.stats[op_idx].as_ref())
                    .map(|s| s.median_us)
                    .fold(f64::INFINITY, f64::min);
                log_sum += (r.stats[op_idx].as_ref().unwrap().median_us / best).ln();
            }
            (r.name, (log_sum / op_indices.len() as f64).exp())
        })
        .collect();
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    for (rank, (name, score)) in scored.iter().enumerate() {
        out.push_str(&format!("| {} | {name} | {score:.2}× |\n", rank + 1));
    }
    out.push('\n');
    out
}

fn render_report(results: &[SuiteResult], cfg: &BenchConfig) -> String {
    let mut out = String::new();
    out.push_str("# Rust Database / ORM Benchmark Results\n\n");
    out.push_str(&format!(
        "Machine-generated by `cargo run --release`. Iterations: {} per single-row op, \
         {} bulk batches of {} rows, {} per read-page op. All timings are per-operation \
         latency on a single connection (median unless noted).\n\n",
        cfg.iters,
        cfg.bulk_iters,
        suites::BULK_SIZE,
        cfg.read_iters
    ));

    // Per-operation tables. Read-only suites appear only in the ops they run.
    for (op_idx, op) in OPS.iter().enumerate() {
        out.push_str(&format!("## {op}\n\n"));
        out.push_str("| Suite | median | mean | p95 | ops/sec |\n");
        out.push_str("|---|---:|---:|---:|---:|\n");
        let best = results
            .iter()
            .filter_map(|r| r.stats[op_idx].as_ref())
            .map(|s| s.median_us)
            .fold(f64::INFINITY, f64::min);
        let mut rows: Vec<&SuiteResult> = results
            .iter()
            .filter(|r| r.stats[op_idx].is_some())
            .collect();
        rows.sort_by(|a, b| {
            a.stats[op_idx]
                .as_ref()
                .unwrap()
                .median_us
                .partial_cmp(&b.stats[op_idx].as_ref().unwrap().median_us)
                .unwrap()
        });
        for r in rows {
            let s = r.stats[op_idx].as_ref().unwrap();
            let marker = if (s.median_us - best).abs() < f64::EPSILON {
                " 🏆"
            } else {
                ""
            };
            out.push_str(&format!(
                "| {}{marker} | {} | {} | {} | {:.0} |\n",
                r.name,
                fmt_us(s.median_us),
                fmt_us(s.mean_us),
                fmt_us(s.p95_us),
                s.ops_per_sec()
            ));
        }
        out.push('\n');
    }

    // Overall ranking: geometric mean of each suite's slowdown vs. the
    // fastest suite per operation, computed separately per database engine
    // (comparing SQLite latencies against Postgres round-trips would be
    // apples to oranges).
    out.push_str("## Overall ranking (geometric mean of relative latency, 1.00 = fastest)\n\n");
    out.push_str("Read-only suites skip the write operations, so they are excluded here and \
                  ranked in the read-only section below.\n\n");
    for engine in ["SQLite", "PostgreSQL"] {
        let group: Vec<&SuiteResult> = results
            .iter()
            .filter(|r| r.name.contains(engine) && r.stats.iter().all(Option::is_some))
            .collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!("### {engine}\n\n"));
        out.push_str(&ranking_table(&group, &(0..OPS.len()).collect::<Vec<_>>()));
    }

    // Read-op-only ranking: every suite runs the reads, so this is where a
    // read-only deployment (embedded, immutable database) should look.
    out.push_str(&format!(
        "## Read-only ranking ({})\n\n",
        READ_OPS.map(|i| OPS[i]).join(", ")
    ));
    for engine in ["SQLite", "PostgreSQL"] {
        let group: Vec<&SuiteResult> = results.iter().filter(|r| r.name.contains(engine)).collect();
        if group.is_empty() {
            continue;
        }
        out.push_str(&format!("### {engine}\n\n"));
        out.push_str(&ranking_table(&group, &READ_OPS));
    }

    // Lines-of-code proxy for developer effort: how much code each suite
    // needed to implement the exact same eight operations.
    out.push_str("## Benchmark implementation size (lines of code for identical operations)\n\n");
    out.push_str("| Suite | LOC | Shared definitions |\n|---|---:|---|\n");
    let loc = |src: &str| src.lines().filter(|l| !l.trim().is_empty()).count();
    let diesel_schema = loc(include_str!("suites/diesel_schema.rs"));
    let seaorm_entities = loc(include_str!("suites/seaorm_entities.rs"));
    let rows = [
        (
            "rusqlite (raw) + SQLite",
            loc(include_str!("suites/rusqlite_sqlite.rs")),
            String::new(),
        ),
        (
            "rusqlite (read-only) + SQLite",
            loc(include_str!("suites/rusqlite_sqlite_readonly.rs")),
            String::new(),
        ),
        (
            "tokio-postgres (raw) + PostgreSQL",
            loc(include_str!("suites/tokio_postgres_pg.rs")),
            String::new(),
        ),
        (
            "SQLx + SQLite",
            loc(include_str!("suites/sqlx_sqlite.rs")),
            String::new(),
        ),
        (
            "SQLx + PostgreSQL",
            loc(include_str!("suites/sqlx_postgres.rs")),
            String::new(),
        ),
        (
            "Diesel + SQLite",
            loc(include_str!("suites/diesel_sqlite.rs")),
            format!("+{diesel_schema} (schema/models)"),
        ),
        (
            "Diesel + PostgreSQL",
            loc(include_str!("suites/diesel_postgres.rs")),
            format!("+{diesel_schema} (schema/models)"),
        ),
        (
            "SeaORM + SQLite",
            loc(include_str!("suites/seaorm_sqlite.rs")),
            format!("+{seaorm_entities} (entities)"),
        ),
        (
            "SeaORM + PostgreSQL",
            loc(include_str!("suites/seaorm_postgres.rs")),
            format!("+{seaorm_entities} (entities)"),
        ),
    ];
    for (name, n, shared) in rows {
        out.push_str(&format!("| {name} | {n} | {shared} |\n"));
    }
    out.push('\n');
    out
}

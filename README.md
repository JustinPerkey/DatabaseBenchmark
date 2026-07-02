# DatabaseBenchmark

A benchmark suite that runs the **same eight database operations** through every
mainstream Rust database access layer, against both **SQLite** and **PostgreSQL**,
and reports per-operation latency plus a developer-experience comparison.

The goal: find the database + ORM combination with the best developer experience
and the most performance.

## What is compared

| Layer | Type | Databases |
|---|---|---|
| [rusqlite](https://crates.io/crates/rusqlite) | raw driver (baseline) | SQLite |
| [tokio-postgres](https://crates.io/crates/tokio-postgres) | raw async driver (baseline) | PostgreSQL |
| [SQLx](https://crates.io/crates/sqlx) | async SQL toolkit (not an ORM) | SQLite, PostgreSQL |
| [Diesel](https://crates.io/crates/diesel) | sync compile-time-checked ORM/query builder | SQLite, PostgreSQL |
| [SeaORM](https://crates.io/crates/sea-orm) | async dynamic ORM (built on SQLx) | SQLite, PostgreSQL |

Every suite implements the same trait ([`src/suites/mod.rs`](src/suites/mod.rs)) and is
seeded with byte-identical data via raw SQL, so the measured differences come from the
access layer itself, not from setup differences.

**Operations measured** (per-operation latency, single connection):

1. `insert_one` — insert a single row
2. `insert_bulk_100` — insert 100 rows in one idiomatic batch
3. `fetch_by_id` — primary-key lookup materialized into a typed struct
4. `fetch_page_50` — filtered + ordered page of 50 rows
5. `join_top_50` — inner join posts→users with filters
6. `update_one` — update one column by primary key
7. `delete_one` — delete one row by primary key

**Fairness rules:** identical schema and seed data everywhere; SQLite always runs
WAL + `synchronous=NORMAL`; Postgres suites all talk to the same local server over
TCP with one connection; every read materializes rows into typed structs; results
are verified (`ensure!`) so no suite can skip work.

## Running it

```bash
# SQLite suites need nothing. For the Postgres suites:
docker run -d -p 5432:5432 -e POSTGRES_USER=bench \
  -e POSTGRES_PASSWORD=bench -e POSTGRES_DB=bench postgres:16

cargo run --release          # full run, writes RESULTS.md
BENCH_QUICK=1 cargo run --release   # fast smoke run
BENCH_ITERS=2000 cargo run --release  # more samples
```

If Postgres isn't reachable the suite automatically runs SQLite-only.

## Results

Full machine-generated tables are in [RESULTS.md](RESULTS.md). Summary from a run on
this repo's reference environment (Linux, PostgreSQL 16, localhost, `--release`,
median latency):

### Performance ranking (geometric mean vs. fastest, lower is better)

| SQLite | | PostgreSQL | |
|---|---:|---|---:|
| rusqlite (raw) | 1.00× | **Diesel** | **1.03×** |
| **Diesel** | **1.26×** | tokio-postgres (raw) | 1.19× |
| SQLx | 19.8× | SQLx | 1.89× |
| SeaORM | 20.5× | SeaORM | 1.97× |

Representative absolute numbers (median):

| Operation | Diesel+SQLite | SQLx+SQLite | Diesel+PG | SQLx+PG | SeaORM+PG |
|---|---:|---:|---:|---:|---:|
| fetch_by_id | 2.3 µs | 176 µs | 66 µs | 251 µs | 274 µs |
| fetch_page_50 | 24 µs | 393 µs | 114 µs | 332 µs | 334 µs |
| insert_one | 8.3 µs | 181 µs | 1.06 ms | 1.31 ms | 1.32 ms |

Key takeaways from the numbers:

- **Diesel is effectively free.** On both engines it benchmarks at raw-driver speed
  (it even beat raw tokio-postgres on reads — sync libpq round-trips have less
  per-call overhead than an async executor on a single connection).
- **SQLx/SeaORM pay a large tax on SQLite (~20×).** sqlx's SQLite driver runs each
  connection on a dedicated background thread and every command crosses a channel,
  so a 2 µs point-read costs ~175 µs. If your database is embedded SQLite, an async
  driver is actively counterproductive.
- **On Postgres the gap compresses** because network round-trips and WAL fsync
  dominate writes (~1 ms), but on reads Diesel is still ~2–4× faster than
  SQLx/SeaORM.
- **SeaORM ≈ SQLx + a little more**, as expected since it's built on SQLx.

### Developer experience scorecard

LOC below is what each layer needed to implement the identical benchmark operations
(from `RESULTS.md`, generated at build time).

| | Diesel | SQLx | SeaORM | raw drivers |
|---|---|---|---|---|
| Benchmark LOC (PG suite) | 93 (+39 shared schema) | 134 | 116 (+52 entities) | 202 |
| Query style | Rust DSL query builder | hand-written SQL | entity/ActiveModel API | hand-written SQL |
| Compile-time query checking | ✅ full, offline | ✅ optional (`query!` needs a live DB or cached metadata) | ⚠️ types only, queries checked at runtime | ❌ |
| Async | ❌ sync (use `deadpool-diesel`/`spawn_blocking` in async servers) | ✅ native | ✅ native | tokio-postgres ✅ / rusqlite ❌ |
| Migrations | ✅ first-class CLI | ✅ `sqlx migrate` | ✅ `sea-orm-cli` + programmatic | ❌ DIY |
| Learning curve | steep (trait-heavy, famously long error messages) | shallow (it's just SQL) | moderate (ActiveModel conventions) | shallow but verbose |
| Escape hatch to raw SQL | ✅ | n/a (it is SQL) | ✅ | n/a |

## Recommendation

**Best performance + best safety: Diesel + PostgreSQL** (or Diesel + SQLite for
embedded/single-node — it's within 26% of raw rusqlite). You get raw-driver
performance, fully compile-time-checked queries, and the least per-operation code —
the LOC table shows the DSL is *more* compact than hand-written SQL once the schema
is declared. The costs are a steeper learning curve and a sync API: in an async web
server you must run it through a blocking pool (`deadpool-diesel`), which is
well-trodden but is real friction.

**Best developer experience for an async-first team: SQLx + PostgreSQL.** You write
plain SQL (nothing to learn, nothing the ORM can't express), get optional
compile-time query verification, and native async. You give up roughly 2× on read
latency vs. Diesel — usually invisible behind network and query cost in a real
service.

**SeaORM** is the choice if you specifically want dynamic, ActiveRecord-style
ergonomics (runtime-composed queries, mutable ActiveModels, built-in
relations/pagination). It benchmarked slowest here and its queries aren't checked at
compile time, so it's not this benchmark's winner on either axis.

**Avoid SQLx/SeaORM with SQLite** in latency-sensitive paths — use Diesel or
rusqlite for embedded databases.

### Which database?

SQLite and Postgres solve different problems, but the numbers frame the tradeoff:
local SQLite point-reads are ~30× faster than a Postgres round-trip and writes are
~100× faster (no network, no per-commit WAL fsync at `synchronous=NORMAL`). If one
process owns the data, **SQLite + Diesel** is unbeatable. The moment you need
concurrent writers, multiple app instances, or Postgres-only SQL features (rich
types, `unnest` bulk loading, mature tooling), **PostgreSQL + Diesel** carries the
same code over — the benchmark's Diesel suites differ only in their connection setup.

## Repository layout

```
src/
├── main.rs                 # orchestration + report generation
├── harness.rs              # warmup/timing/percentile machinery
└── suites/
    ├── mod.rs              # Suite trait, shared DDL + seed data
    ├── rusqlite_sqlite.rs
    ├── tokio_postgres_pg.rs
    ├── sqlx_sqlite.rs
    ├── sqlx_postgres.rs
    ├── diesel_schema.rs    # shared Diesel table!/model definitions
    ├── diesel_sqlite.rs
    ├── diesel_postgres.rs
    ├── seaorm_entities.rs  # shared SeaORM entity definitions
    ├── seaorm_sqlite.rs
    └── seaorm_postgres.rs
```

## Caveats

- Single-connection latency is the metric. Throughput under concurrency would favor
  the async stacks more; add a concurrent scenario before generalizing to high-QPS
  services.
- Postgres write latency is dominated by WAL fsync of the local server, which
  compresses differences between layers on insert/update/delete.
- SQLx was measured with runtime queries (`query`/`query_as`); the `query!` macros
  add compile-time checking but identical runtime behavior.
- MySQL/MariaDB and libraries like `sea-query`-only, `cornucopia`, or `welds` are
  not (yet) included.

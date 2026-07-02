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
| [duckdb](https://crates.io/crates/duckdb) | raw driver | DuckDB (embedded OLAP) |
| [redb](https://crates.io/crates/redb) | pure-Rust embedded key-value store | redb |

DuckDB and redb are the purpose-built alternatives: DuckDB is a read-optimized
embedded *analytics* engine, redb is a transactional embedded KV store whose sweet
spot is exactly read-heavy point lookups. redb has no SQL — its suite hand-rolls
filtering, ordering, and the join in application code over bincode-serialized
structs, which is part of the developer-experience comparison.

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
# Embedded suites (SQLite, DuckDB, redb) need nothing. For the Postgres suites:
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

| Embedded engines | | PostgreSQL | |
|---|---:|---|---:|
| rusqlite (raw) + SQLite | 1.21× | **Diesel** | **1.00×** |
| **Diesel + SQLite** | **1.46×** | tokio-postgres (raw) | 1.36× |
| redb (embedded KV) | 14.9× | SeaORM | 2.01× |
| SQLx + SQLite | 22.1× | SQLx | 2.09× |
| SeaORM + SQLite | 22.2× | |
| duckdb-rs + DuckDB | 147.7× | |

Representative absolute numbers (median):

| Operation | redb | Diesel+SQLite | SQLx+SQLite | DuckDB | Diesel+PG | SQLx+PG |
|---|---:|---:|---:|---:|---:|---:|
| fetch_by_id | **0.9 µs** | 2.3 µs | 169 µs | 354 µs | 67 µs | 261 µs |
| fetch_page_50 | **12 µs** | 23 µs | 349 µs | 2.40 ms | 118 µs | 355 µs |
| join_top_50 | 35 µs | **24 µs** | 297 µs | 1.52 ms | 165 µs | 392 µs |
| insert_one | 1.06 ms | **8.5 µs** | 163 µs | 1.51 ms | 1.19 ms | 1.99 ms |

Key takeaways from the numbers:

- **Diesel is effectively free.** On both engines it benchmarks at raw-driver speed
  (it even beat raw tokio-postgres on reads — sync libpq round-trips have less
  per-call overhead than an async executor on a single connection).
- **SQLx/SeaORM pay a large tax on SQLite (~20×).** sqlx's SQLite driver runs each
  connection on a dedicated background thread and every command crosses a channel,
  so a 2 µs point-read costs ~170 µs. If your database is embedded SQLite, an async
  driver is actively counterproductive.
- **On Postgres the gap compresses** because network round-trips and WAL fsync
  dominate writes (~1 ms), but on reads Diesel is still ~2–4× faster than
  SQLx/SeaORM.
- **SeaORM ≈ SQLx + a little more**, as expected since it's built on SQLx.
- **redb is the fastest reader, by a lot** — sub-microsecond point lookups (2× raw
  SQLite) and the fastest page scan. The costs: every commit is expensive (~1 ms
  even with relaxed durability, so batch your writes), and there is no SQL — the
  join and filters in its suite are hand-written application code.
- **DuckDB is the wrong purpose-built tool for this shape of workload.** It's a
  columnar OLAP engine: point lookups, small pages, and row-at-a-time writes are
  its worst case (147× slowdown). It wins at large aggregations/scans, which this
  OLTP-shaped suite deliberately doesn't measure — see caveats.

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

### Read-mostly workloads and hand-editing the data

For a workload that mostly reads and occasionally needs a human to inspect or fix
the data by hand, the engines differ more in tooling than in speed:

| | SQLite | DuckDB | PostgreSQL | redb |
|---|---|---|---|---|
| Read latency (this suite) | excellent | poor for row lookups | good (network-bound) | best |
| Hand-edit tooling | **best**: `sqlite3` CLI, [DB Browser for SQLite](https://sqlitebrowser.org/), countless GUIs — open the file, edit, save | `duckdb` CLI is excellent; can also query/edit data living in **plain CSV/JSON/Parquet files** | `psql`/pgAdmin — full power, but a server must be running | **none** — binary format, you must write Rust to touch it |
| Concurrent read scaling | great (WAL: many readers) | great for readers | great | great (MVCC snapshots) |
| Extra safety for read-mostly | open connections with `?mode=ro` or `immutable=1` | `Config::access_mode(ReadOnly)` | `default_transaction_read_only = on` | read-only txns are the default API |

Two patterns worth knowing if hand-editability is a priority:

- **SQLite as the editable source of truth.** One file, edit it in a GUI, every
  Rust access layer in this suite can read it, and read-only connections make the
  read path immune to accidental writes.
- **DuckDB over CSV/JSON.** DuckDB can query text files directly
  (`SELECT * FROM 'users.csv'`), so the "database" can literally be files you edit
  in a text editor — at the price of the row-lookup latencies in the tables above.

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

**For read-mostly data that humans hand-edit: SQLite + Diesel** (or plain
rusqlite). It pairs near-redb read speed with the best hand-editing tooling of any
engine here. Reach for **redb** only when reads are pure key lookups, every
microsecond counts, and nobody needs to open the data in a tool; reach for
**DuckDB** only when the reads are aggregations over large data (or you want the
data to live in hand-editable CSV/JSON) — for row-oriented reads it was the
slowest engine in the suite.

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
    ├── duckdb_duckdb.rs
    ├── redb_redb.rs
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
- The workload is OLTP-shaped (point reads, small pages, row writes). That is
  DuckDB's worst case; its purpose is aggregations/scans over millions of rows,
  which this suite does not measure. Don't read its ranking as "DuckDB is slow" —
  read it as "DuckDB is not an OLTP store."
- redb write transactions run with `Durability::Eventual` to match the SQLite
  suites' `synchronous=NORMAL` (no per-commit fsync). Even so, its copy-on-write
  commits cost ~1 ms — redb wants batched writes.
- MySQL/MariaDB, LMDB (`heed`), `sled`, `fjall`, and libraries like
  `sea-query`-only, `cornucopia`, or `welds` are not (yet) included.

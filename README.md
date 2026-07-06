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
| rusqlite (read-only) | raw driver, `SQLITE_OPEN_READ_ONLY` + `immutable=1` | SQLite |
| rusqlite (in-memory) | raw driver, `:memory:` database loaded from disk via the backup API | SQLite |
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

The read-only suite runs only the three read operations (`fetch_by_id`,
`fetch_page_50`, `join_top_50`); the harness skips writes for it and it is ranked
separately. It models a database deployed on a read-only filesystem (embedded
Linux, squashfs): the file is seeded by a writable connection, then reopened with
`SQLITE_OPEN_READ_ONLY` and `immutable=1`, which skips all locking and change
detection and never creates journal/WAL files.

The in-memory suite measures the impact of loading the whole database into RAM:
the same seeded file is copied page-by-page into a `:memory:` connection with the
SQLite backup API (the one-time load cost is printed during the run), then all
seven operations run against memory only — no filesystem, no fsync, no journal
files. Writes are not durable: they die with the process.

**Fairness rules:** identical schema and seed data everywhere; writable on-disk SQLite
suites run WAL + `synchronous=NORMAL`; Postgres suites all talk to the same local server over
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
| **rusqlite (in-memory)** | **1.00×** | **Diesel** | **1.00×** |
| rusqlite (raw, WAL) | 2.50× | tokio-postgres (raw) | 1.25× |
| Diesel | 3.18× | SQLx | 1.95× |
| SQLx | 62.8× | SeaORM | 2.05× |
| SeaORM | 65.9× | | |

The read-only suite skips writes, so it is ranked separately over the three read
operations (from RESULTS.md):

| SQLite, reads only | |
|---|---:|
| **rusqlite (read-only, immutable)** | **1.02×** |
| **rusqlite (in-memory)** | **1.02×** |
| rusqlite (raw, WAL) | 1.44× |
| Diesel | 1.67× |
| SeaORM | 52.7× |
| SQLx | 53.6× |

Representative absolute numbers (median):

| Operation | rusqlite in-memory | rusqlite read-only | Diesel+SQLite | SQLx+SQLite | Diesel+PG | SQLx+PG |
|---|---:|---:|---:|---:|---:|---:|
| fetch_by_id | 0.7 µs | 0.7 µs | 1.7 µs | 182 µs | 70 µs | 282 µs |
| fetch_page_50 | 14 µs | 14 µs | 20 µs | 392 µs | 114 µs | 341 µs |
| insert_one | 1.0 µs | n/a | 7.2 µs | 186 µs | 960 µs | 1.35 ms |

Key takeaways from the numbers:

- **Loading the database in memory speeds up writes ~5–6× but barely moves
  reads.** `insert_one` drops from 6.4 µs (WAL on disk) to 1.0 µs, `update_one`
  from 5.6 µs to 1.1 µs, `delete_one` from 5.5 µs to 1.0 µs — nothing touches the
  filesystem, so all journal and sync work disappears. Reads land exactly on the
  read-only immutable numbers (0.7 µs point-read): a warm on-disk database is
  already served from the OS page cache, so RAM residency itself buys nothing —
  the ~1.4× read win over WAL comes from skipping locking/change-detection, which
  `immutable=1` achieves without giving up the on-disk file. The one-time load
  cost is trivial at this size (0.4 MiB in 0.1 ms, via the backup API) and scales
  linearly. The price: writes are not durable — the database dies with the process.
- **Diesel is effectively free.** On both engines it benchmarks at raw-driver speed
  (it even beat raw tokio-postgres on reads — sync libpq round-trips have less
  per-call overhead than an async executor on a single connection).
- **Read-only immutable SQLite matches in-memory on reads while keeping the file
  on disk, ~1.4× faster than WAL.** `immutable=1` tells SQLite the file cannot
  change, so it skips per-query locking and change detection entirely — a
  point-read drops from 1.6 µs to 0.7 µs. It also works on a read-only mount,
  where WAL cannot even open.
- **SQLx/SeaORM pay a large tax on SQLite (~25× vs. the raw driver).** sqlx's
  SQLite driver runs each connection on a dedicated background thread and every
  command crosses a channel, so a 2 µs point-read costs ~185 µs. If your database
  is embedded SQLite, an async driver is actively counterproductive.
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
embedded/single-node — it's within ~27% of raw rusqlite). You get raw-driver
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

**Read-only embedded deployments (read-only rootfs, squashfs): open SQLite with
`SQLITE_OPEN_READ_ONLY` and `immutable=1`.** It ties in-memory for the fastest
reads measured (~1.4× faster than WAL) while keeping the file on disk, needs no
journal or lock files, and the database file stays hand-editable offline with any
SQLite tool before it is baked into the image.

**Load the database in memory only when you need fast *writes* on ephemeral
data.** That's where the impact is: writes get ~5–6× faster because nothing is
journaled or synced. For read speed alone, memory residency buys nothing over
`immutable=1` (or even a warm page cache) — don't give up durability for it. Good
fits are caches, session stores, queues of recomputable work, and test fixtures;
the backup API loads the seed file at ~4 GiB/s here, and can also snapshot the
memory database back to disk periodically if losing the last few minutes is
acceptable.

### Which database?

SQLite and Postgres solve different problems, but the numbers frame the tradeoff:
local SQLite point-reads are ~40× faster than a Postgres round-trip and writes are
~150× faster (no network, no per-commit WAL fsync at `synchronous=NORMAL`). If one
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
    ├── rusqlite_sqlite_readonly.rs  # SQLITE_OPEN_READ_ONLY + immutable=1, reads only
    ├── rusqlite_sqlite_memory.rs    # :memory: database loaded from disk via backup API
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
- The seeded database is small (~0.4 MiB), so on-disk reads are always served from
  the OS page cache. On a dataset larger than RAM (or a cold cache), the in-memory
  suite's read advantage would grow — but so would its load time and memory bill.
- Postgres write latency is dominated by WAL fsync of the local server, which
  compresses differences between layers on insert/update/delete.
- SQLx was measured with runtime queries (`query`/`query_as`); the `query!` macros
  add compile-time checking but identical runtime behavior.
- MySQL/MariaDB and libraries like `sea-query`-only, `cornucopia`, or `welds` are
  not (yet) included.

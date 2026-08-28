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
| Diesel (in-memory) | same Diesel code against a `:memory:` database loaded from disk | SQLite |
| [diesel-async](https://crates.io/crates/diesel-async) | async connections for Diesel's DSL — native on PostgreSQL, `spawn_blocking` on SQLite | SQLite, PostgreSQL |
| [SeaORM](https://crates.io/crates/sea-orm) 2.0 | async dynamic ORM (built on SQLx) | SQLite, PostgreSQL |

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

Two in-memory suites measure the impact of loading the whole database into RAM,
one at the raw-driver level and one through an ORM. Both seed the same file on
disk first, then load it into a `:memory:` connection (the one-time load cost is
printed during the run) and run all seven operations against memory only — no
filesystem, no fsync, no journal files. Writes are not durable: they die with the
process. They differ only in how the load happens: rusqlite copies the file
page-by-page with the SQLite backup API, while Diesel — which exposes no backup
handle, and whose `deserialize_readonly_database_from_buffer` would give up
writes — attaches the seeded file and copies it with `INSERT ... SELECT`. The
Diesel in-memory suite runs byte-identical query code to the on-disk Diesel
suite, so the difference between them is purely the storage medium.

The two diesel-async suites run byte-identical query code to the sync Diesel
suites — same `table!` definitions, same models, same DSL, only the connection
type and the `.await`s differ — so the delta between each pair is the cost of
the async connection alone. On PostgreSQL that is a native `AsyncPgConnection`
(tokio-postgres instead of libpq); on SQLite, which has no async protocol, it is
`SyncConnectionWrapper<SqliteConnection>`, which moves every statement onto
`tokio::task::spawn_blocking`. One operation is *not* like-for-like there:
diesel-async cannot express Diesel's single-statement multi-row SQLite insert, so
its `insert_bulk_100` does 100 inserts in one transaction — see
[diesel-async](#diesel-async) below.

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
this repo's reference environment (Linux, PostgreSQL 16 on localhost, `--release`,
median latency). All suites were re-measured together in the run that produced the
current RESULTS.md, so the numbers below are internally comparable; absolute values
shifted from earlier revisions of this file because the reference machine changed,
and the Postgres round-trip cost in particular is machine- and load-sensitive, so
compare the columns of one run against each other rather than against an older one.

### Performance ranking (geometric mean vs. fastest, lower is better)

| SQLite | | PostgreSQL | |
|---|---:|---|---:|
| **rusqlite (in-memory)** | **1.06×** | **Diesel** | **1.06×** |
| **Diesel (in-memory)** | **1.54×** | tokio-postgres (raw) | 1.18× |
| rusqlite (raw, WAL) | 2.45× | **Diesel-async** | **1.68×** |
| Diesel | 3.22× | SQLx | 2.10× |
| Diesel-async | 27.8×\* | SeaORM | 2.42× |
| SQLx | 55.8× | | |
| SeaORM | 57.5× | | |

\* Diesel-async + SQLite carries the non-comparable bulk-insert row described
below; without it the same geometric mean is 22.9×.

The read-only suite skips writes, so it is ranked separately over the three read
operations (from RESULTS.md):

| SQLite, reads only | | PostgreSQL, reads only | |
|---|---:|---|---:|
| **rusqlite (read-only, immutable)** | **1.01×** | **Diesel** | **1.00×** |
| **Diesel (in-memory)** | **1.20×** | tokio-postgres (raw) | 1.42× |
| **rusqlite (in-memory)** | **1.22×** | **Diesel-async** | **1.64×** |
| rusqlite (raw, WAL) | 1.48× | SQLx | 2.79× |
| Diesel | 1.79× | SeaORM | 3.28× |
| Diesel-async | 13.4× | | |
| SQLx | 51.1× | | |
| SeaORM | 53.9× | | |

Representative absolute numbers (median):

| Operation | rusqlite in-memory | Diesel in-memory | rusqlite read-only | Diesel+SQLite | Diesel-async+SQLite | SQLx+SQLite |
|---|---:|---:|---:|---:|---:|---:|
| fetch_by_id | 1.4 µs | 0.9 µs | 0.8 µs | 2.3 µs | 46 µs | 179 µs |
| fetch_page_50 | 16 µs | 21 µs | 15 µs | 23 µs | 114 µs | 433 µs |
| insert_one | 1.3 µs | 2.3 µs | n/a | 8.6 µs | 61 µs | 188 µs |

| Operation | tokio-postgres | Diesel+PG | Diesel-async+PG | SQLx+PG | SeaORM+PG |
|---|---:|---:|---:|---:|---:|
| fetch_by_id | 120 µs | 97 µs | 145 µs | 300 µs | 357 µs |
| fetch_page_50 | 203 µs | 142 µs | 236 µs | 392 µs | 433 µs |
| insert_one | 350 µs | 359 µs | 617 µs | 626 µs | 706 µs |

Key takeaways from the numbers:

- **Loading the database in memory speeds up writes ~5–6× and reads much less.**
  With rusqlite, `insert_one` drops from 7.5 µs (WAL on disk) to 1.3 µs,
  `update_one` from 7.0 µs to 1.4 µs, `delete_one` from 6.7 µs to 1.4 µs — nothing
  touches the filesystem, so all journal and sync work disappears. Reads land on
  the read-only immutable numbers (0.8 µs point-read, 16 µs page): a warm on-disk
  database is already served from the OS page cache, so most of the read win comes
  from skipping locking/change-detection, which `immutable=1` achieves without
  giving up the on-disk file. The one-time load cost is trivial at this size
  (0.4 MiB in 0.1–0.3 ms, via the backup API) and scales linearly. The price:
  writes are not durable — the database dies with the process.
- **The in-memory win is not a raw-driver privilege: Diesel keeps it.** Moving the
  same Diesel code from WAL-on-disk to `:memory:` cuts overall latency ~2×
  (3.22× → 1.54× on the SQLite ranking) — single-row writes get ~3–5× faster
  (`insert_one` 8.6 → 2.3 µs, `delete_one` 6.7 → 1.4 µs), point reads ~2.5×
  (2.3 → 0.9 µs), and page/join reads ~1.1×. The ORM keeps its usual ~1.6× tax
  over rusqlite, and on the read-only ranking in-memory Diesel (1.20×) still comes
  out ahead of the raw driver on disk (1.48×) — though on the 50-row page scan
  specifically it is a hair slower, so the win is in point reads, not scans.
  What costs more is the load: Diesel has no backup-API handle, so the
  suite attaches the seeded file and replays it with `INSERT ... SELECT`, which
  rebuilds every B-tree instead of copying pages — 5–8 ms versus 0.1–0.3 ms for
  the same 0.4 MiB. Still a one-time cost, but budget for it (or load with
  rusqlite and hand Diesel a shared-cache memory URI) if the dataset is large.
- **Diesel is effectively free.** On both engines it benchmarks at raw-driver speed
  (it even beat raw tokio-postgres on reads — sync libpq round-trips have less
  per-call overhead than an async executor on a single connection).
- **Read-only immutable SQLite matches in-memory on reads while keeping the file
  on disk, ~1.4× faster than WAL.** `immutable=1` tells SQLite the file cannot
  change, so it skips per-query locking and change detection entirely — a
  point-read drops from 2.0 µs to 0.8 µs. It also works on a read-only mount,
  where WAL cannot even open.
- **SQLx/SeaORM pay a large tax on SQLite (~23× vs. the raw driver overall,
  ~90× on a point read).** sqlx's SQLite driver runs each connection on a
  dedicated background thread and every command crosses a channel, so a 2 µs
  point-read costs ~180 µs. If your database
  is embedded SQLite, an async driver is actively counterproductive.
- **Async Diesel is much cheaper than async SQL toolkits, on both engines.** On
  Postgres diesel-async (1.68×) sits between raw tokio-postgres (1.18×) and SQLx
  (2.10×) while sending byte-identical SQL to sync Diesel; on SQLite the
  `spawn_blocking` hop costs ~20× on a point read (8.6× on the overall SQLite
  ranking) against sync Diesel, but is still ~2× cheaper than sqlx's
  channel-per-command SQLite driver. See [diesel-async](#diesel-async).
- **On Postgres the gap compresses** because network round-trips and WAL fsync
  dominate writes, but on reads Diesel is still ~2.5–3× faster than SQLx/SeaORM.
- **SeaORM ≈ SQLx + a little more**, as expected since it's built on SQLx.

### Developer experience scorecard

LOC below is what each layer needed to implement the identical benchmark operations
(from `RESULTS.md`, generated at build time).

| | Diesel | diesel-async | SQLx | SeaORM | raw drivers |
|---|---|---|---|---|---|
| Benchmark LOC (PG suite) | 93 (+39 shared schema) | 123 (+39, the same schema file) | 134 | 116 (+52 entities) | 202 |
| Query style | Rust DSL query builder | the same DSL, `.await`ed | hand-written SQL | entity/ActiveModel API | hand-written SQL |
| Compile-time query checking | ✅ full, offline | ✅ full, offline | ✅ optional (`query!` needs a live DB or cached metadata) | ⚠️ types only, queries checked at runtime | ❌ |
| Async | ❌ sync (use `deadpool-diesel`/`spawn_blocking` in async servers) | ✅ native on PG/MySQL, `spawn_blocking` on SQLite | ✅ native | ✅ native | tokio-postgres ✅ / rusqlite ❌ |
| Migrations | ✅ first-class CLI | ✅ same CLI (`migrations` feature for programmatic runs) | ✅ `sqlx migrate` | ✅ `sea-orm-cli` + programmatic | ❌ DIY |
| Learning curve | steep (trait-heavy, famously long error messages) | Diesel's, plus async trait bounds in the errors | shallow (it's just SQL) | moderate (ActiveModel conventions) | shallow but verbose |
| Escape hatch to raw SQL | ✅ | ✅ | n/a (it is SQL) | ✅ | n/a |
| Bulk insert on SQLite | ✅ one statement | ❌ does not compile (see below) | ✅ | ✅ | ✅ |

## diesel-async

[diesel-async](https://crates.io/crates/diesel-async) 0.9.2 puts Diesel's query
DSL behind `async fn`. Two suites cover it, and both run byte-identical query
code to their sync Diesel counterparts — only the connection type and the
`.await`s differ:

- **Diesel-async + PostgreSQL** — `AsyncPgConnection`, a native async connection
  that speaks the Postgres wire protocol through `tokio-postgres` instead of libpq.
- **Diesel-async + SQLite** — `SyncConnectionWrapper<SqliteConnection>`. SQLite has
  no async protocol, so diesel-async wraps the ordinary sync connection and moves
  every statement onto `tokio::task::spawn_blocking`.

### Adoption cost: one dependency and a Diesel minor bump

`diesel-async = "0.9"` requires `diesel ~2.3`, so `diesel = "2.2"` became `"2.3"`
in `Cargo.toml`. That was the entire migration: no existing suite needed a source
change for 2.3, and `diesel_schema.rs` — the `table!` definitions and the
`Queryable`/`Insertable` models — is shared with the async suites unmodified. The
same schema, models, and DSL expressions typecheck against both connection types,
which is the main practical argument for diesel-async over a second query layer.

### PostgreSQL: identical SQL, ~1.6× the latency of sync Diesel

With `log_statement=all` on the server, the statements diesel-async sends are
byte-identical to sync Diesel's — same text, same placeholders, same session
setup (`SET TIME ZONE 'UTC'`, `SET CLIENT_ENCODING TO 'UTF8'`), same count per
operation. The queries are not the variable here:

| op (median) | tokio-postgres | Diesel + PG | Diesel-async + PG | SQLx + PG |
|---|---:|---:|---:|---:|
| insert_one | 350 µs | 359 µs | 617 µs | 626 µs |
| insert_bulk_100 | 781 µs | 985 µs | 1465 µs | 1087 µs |
| fetch_by_id | 120 µs | 97 µs | 145 µs | 300 µs |
| fetch_page_50 | 203 µs | 142 µs | 236 µs | 392 µs |
| join_top_50 | 303 µs | 187 µs | 332 µs | 475 µs |
| update_one | 341 µs | 388 µs | 640 µs | 593 µs |
| delete_one | 343 µs | 315 µs | 425 µs | 613 µs |

diesel-async lands between the raw async driver and SQLx on every operation:
1.1–1.9× raw tokio-postgres, and 1.68× overall against sync Diesel's 1.06×. Versus
SQLx it wins the reads clearly (1.4–2× faster) and ties the single-row writes
(within ~10%, where WAL fsync dominates); the one place it loses is the 100-row
batch, 1.35× slower than SQLx and 1.5× slower than sync Diesel on identical SQL.

**Part of the write gap is an extra round trip, not async overhead.** Both layers
cache the same statements — the two `SELECT`s, the join, and the `DELETE` are
prepared once and reused — and both treat `insert_one` and `update_one` as
non-cacheable. What differs is what "non-cacheable" costs. Sync Diesel re-parses
them into libpq's *unnamed* statement, which goes out in a single round trip; the
server log shows one `execute <unnamed>` per call. diesel-async allocates a fresh
*named* prepared statement for every execution (`s0`, `s1`, `s2`, …: 55 executions
of `insert_one` produced 55 statements, 45 `update_one`s produced 45), so each one
pays a `prepare` round trip first. The numbers follow that split — the write it
does reuse, `delete_one`, is 1.35× sync Diesel, while the two it re-prepares are
1.72× and 1.65×. Explicitly calling
`set_prepared_statement_cache_size(CacheSize::Unbounded)` changes nothing: caching
is already on, these queries just are not eligible for it.

### SQLite: correct, but `spawn_blocking` costs ~20×

Wrapping the sync connection works and every query in the suite is the sync
suite's query verbatim, but each statement now round-trips through the blocking
pool: a 2.3 µs point read becomes 46 µs, an 8.6 µs insert becomes 61 µs, and the
overall SQLite ranking goes 3.22× → 27.8×. It is still ~2× cheaper than
SQLx/SeaORM on SQLite (55.8×/57.5×), because one `spawn_blocking` hop beats
sqlx's dedicated connection thread with a channel per command — but against sync
Diesel on the same file it is a pure loss. If a SQLite-backed async service needs
Diesel, `deadpool-diesel` (one `spawn_blocking` per *unit of work*, not per
statement) is the cheaper shape.

### The one thing that does not compile: batch insert on SQLite

`insert_into(users::table).values(&vec).execute(&mut conn).await` is rejected
against `SyncConnectionWrapper<SqliteConnection>`:

```
error[E0271]: type mismatch resolving
  `<Sqlite as SqlDialect>::InsertWithDefaultKeyword == IsoSqlDefaultKeyword`
   = note: required for `BatchInsert<...>` to implement `CanInsertInSingleQuery<Sqlite>`
```

The generic multi-row `VALUES` path requires a backend with the SQL `DEFAULT`
keyword, which SQLite does not have. Sync Diesel still emits one multi-row
statement there, but only through a `SqliteConnection`-specialized `ExecuteDsl`
impl (`SqliteBatchInsertWrapper`) that diesel-async does not implement — so the
capability is not reachable from the async connection. The suite falls back to
the idiomatic alternative, 100 single-row inserts inside one transaction, which
costs 5.3 ms against sync Diesel's 213 µs for the same 100 rows. **Read that row
as a limitation marker, not a measurement**: it is a different amount of work.
Dropping it from the geometric mean moves Diesel-async + SQLite from 27.8× to
22.9×, so it inflates the ranking but is not what makes the suite slow — the
per-statement hop is. PostgreSQL is unaffected; its batch insert is one
multi-row statement, identical to sync Diesel's.

### Verdict

**On PostgreSQL, diesel-async is the async layer to reach for if you want
Diesel's compile-time-checked DSL in an async service.** It costs ~1.6× sync
Diesel's latency but beats SQLx overall (1.68× vs 2.10×) while keeping full
offline query checking, and it shares its schema and models with sync Diesel code
byte for byte. The alternative — sync Diesel behind `deadpool-diesel` — keeps the
faster libpq path at the cost of a blocking pool to size and manage; the numbers
here say the pool is worth it for write-heavy workloads and diesel-async is worth
it for read-heavy ones.

**On SQLite, do not use it for per-statement access.** There is no async SQLite
protocol to win with, batch insert does not compile, and every statement pays for
a thread hop. Use sync Diesel directly, or `deadpool-diesel` if the surrounding
service is async.

## SeaORM 2.0 upgrade

The benchmark now runs **SeaORM 2.0** (released 2026-07-19). It was tested against
1.1.20 before switching, both for migration cost and for regressions.

### Migration cost: zero source changes

The only edit was the version string in `Cargo.toml`. `seaorm_entities.rs`,
`seaorm_sqlite.rs` and `seaorm_postgres.rs` compile untouched, without warnings.
The 2.0 breaking changes that could plausibly have hit this code did not:

| Breaking change | Why it didn't bite |
|---|---|
| `insert_many` overhauled (`InsertMany`, `last_insert_id` now `Option<Value>`) | `insert_many(rows).exec(db)` still infers and still returns a usable result; the new API only matters if you read `last_insert_id` or pass a possibly-empty iterator |
| `delete_by_id` returns `ValidatedDeleteOne` instead of `DeleteMany` | `.exec()` is unchanged; only `exec_with_returning` changes shape |
| `execute`/`query_one`/`query_all` now take SeaQuery statements, with `*_raw` for SQL | the suites use `execute_unprepared`, which is unchanged |
| `DatabaseConnection` became a struct (enum moved to `.inner`) | never matched on |
| SeaQuery 1.0 wants `use sea_orm::ExprTrait` in scope | `ColumnTrait::eq` and `Expr::value` cover everything used here |

What does change underneath: sea-orm 2.0 pulls **SQLx 0.9** and **SeaQuery 1.0**,
is built on **edition 2024**, and raises the MSRV to **Rust 1.94**. Because the
SQLx suites still pin 0.8, the binary links both SQLx majors side by side — fine
for Cargo, but worth knowing if you plan to share types between an ORM and a
hand-rolled SQLx layer.

### Behaviour: identical SQL

With `log_statement=all` on the Postgres server, the 41 distinct statements the
benchmark emitted at the time are byte-identical between 1.1.20 and 2.0.0. SeaORM
still sends one multi-row `VALUES` list for `insert_many`, `RETURNING "id"` for
`insert_one`, and the same aliased `A_*`/`B_*` projection for
`find_also_related`. Nothing about the generated queries changed.

### Performance: no measurable change

Both versions were built as separate binaries and run interleaved, 40 full runs
each (20 in each ordering, to cancel order bias). Numbers are the median across
runs of each run's median per-operation latency. This A/B ran in a slower
container than the reference environment used for the tables above, so read the
columns against each other, not against [RESULTS.md](RESULTS.md) — those tables
are still from the reference machine, and the equivalence shown here is what
makes them valid for 2.0:

| op | SeaORM + SQLite | | | SeaORM + PostgreSQL | | |
|---|---:|---:|---:|---:|---:|---:|
| | 1.1.20 | 2.0.0 | Δ | 1.1.20 | 2.0.0 | Δ |
| insert_one | 277.4 µs | 276.2 µs | −0.4% | 1330 µs | 1361 µs | +2.3% |
| insert_bulk_100 | 360.0 µs | 399.0 µs | +10.8%* | 1886 µs | 1903 µs | +0.9% |
| fetch_by_id | 261.8 µs | 260.4 µs | −0.5% | 422 µs | 417 µs | −1.3% |
| fetch_page_50 | 353.4 µs | 332.3 µs | −6.0% | 504 µs | 499 µs | −1.0% |
| join_top_50 | 408.8 µs | 393.7 µs | −3.7% | 704 µs | 702 µs | −0.3% |
| update_one | 271.9 µs | 270.1 µs | −0.7% | 1345 µs | 1353 µs | +0.6% |
| delete_one | 272.6 µs | 268.2 µs | −1.6% | 1378 µs | 1384 µs | +0.4% |

\* **Not a SeaORM regression.** Every SQLite bulk insert in the 2.0 binary got
slower, including Diesel's (+6.9%) and SQLx's (+3.2%) — suites whose code and
dependencies are byte-identical in both binaries — so it tracks the process, not
the ORM. Re-running `insert_many` on its own in a standalone crate (nothing else
linked, 15 interleaved rounds) gives 320.5 µs on 1.1 vs 327.4 µs on 2.0 by median
and 296.9 µs vs 290.6 µs by minimum: ±2%, i.e. nothing.

The noise floor for the async suites here is roughly ±10%: the same control
comparison shows SQLx + PostgreSQL "improving" 11% between two identical binaries.
The sync suites are the stable reference — rusqlite reproduces to 0.0% on every
operation. Read the SeaORM deltas above against that background, and the
conclusion is that **2.0 is a drop-in upgrade with performance indistinguishable
from 1.1** for these seven operations.

## Recommendation

**Best performance + best safety: Diesel + PostgreSQL** (or Diesel + SQLite for
embedded/single-node — it's within ~31% of raw rusqlite). You get raw-driver
performance, fully compile-time-checked queries, and the least per-operation code —
the LOC table shows the DSL is *more* compact than hand-written SQL once the schema
is declared. The costs are a steeper learning curve and a sync API: in an async web
server you run it through a blocking pool (`deadpool-diesel`) or through
diesel-async, both well-trodden but real friction.

**Best for an async-first team on Postgres: diesel-async.** It keeps the DSL, the
schema, the models, and the full offline compile-time checking, sends byte-identical
SQL, and still ranks ahead of SQLx (1.68× vs 2.10×) — clearly ahead on reads, level
on single-row writes. Budget ~1.6× sync Diesel's latency for the async transport,
and check that nothing in your workload depends on multi-row SQLite inserts.

**Best developer experience if you'd rather write SQL: SQLx + PostgreSQL.** You write
plain SQL (nothing to learn, nothing the ORM can't express), get optional
compile-time query verification, and native async. You give up roughly 2–3× on read
latency vs. Diesel — usually invisible behind network and query cost in a real
service.

**SeaORM** is the choice if you specifically want dynamic, ActiveRecord-style
ergonomics (runtime-composed queries, mutable ActiveModels, built-in
relations/pagination). It benchmarked slowest here and its queries aren't checked at
compile time, so it's not this benchmark's winner on either axis.

**Avoid every async layer with SQLite** in latency-sensitive paths — SQLx and SeaORM
most of all (~50× the raw driver), but diesel-async's `spawn_blocking` wrapper too
(~13× on reads). Use Diesel or rusqlite for embedded databases, and if the service
around them is async, hop to a blocking pool once per unit of work rather than once
per statement.

**Read-only embedded deployments (read-only rootfs, squashfs): open SQLite with
`SQLITE_OPEN_READ_ONLY` and `immutable=1`.** It ties in-memory for the fastest
reads measured (~1.4× faster than WAL) while keeping the file on disk, needs no
journal or lock files, and the database file stays hand-editable offline with any
SQLite tool before it is baked into the image.

**Load the database in memory only when you need fast *writes* on ephemeral
data.** That's where the impact is: writes get ~5–6× faster because nothing is
journaled or synced. For read speed alone, memory residency buys little over
`immutable=1` (or even a warm page cache) — don't give up durability for it. Good
fits are caches, session stores, queues of recomputable work, and test fixtures;
the backup API loads the seed file at ~4 GiB/s here, and can also snapshot the
memory database back to disk periodically if losing the last few minutes is
acceptable. **You do not have to drop to a raw driver to get this**: the Diesel
in-memory suite runs byte-identical query code to the on-disk one and keeps most
of the win, so an ORM is a fine front end for a RAM-resident SQLite database —
just load it with the backup API (via rusqlite, sharing a `file:name?mode=memory&
cache=shared` URI) rather than `INSERT ... SELECT` if the load time matters.

### Which database?

SQLite and Postgres solve different problems, but the numbers frame the tradeoff:
local SQLite point-reads are ~40× faster than a Postgres round-trip and writes are
~40× faster (no network, no per-commit WAL fsync at `synchronous=NORMAL`). If one
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
    ├── diesel_sqlite_memory.rs  # :memory: database loaded from disk via ATTACH + INSERT SELECT
    ├── diesel_postgres.rs
    ├── diesel_async_sqlite.rs   # SyncConnectionWrapper<SqliteConnection> (spawn_blocking)
    ├── diesel_async_postgres.rs # AsyncPgConnection (tokio-postgres transport)
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
  suites' read advantage would grow — but so would their load time and memory bill.
- Postgres write latency is dominated by WAL fsync of the local server, which
  compresses differences between layers on insert/update/delete.
- SQLx was measured with runtime queries (`query`/`query_as`); the `query!` macros
  add compile-time checking but identical runtime behavior.
- MySQL/MariaDB and libraries like `sea-query`-only, `cornucopia`, or `welds` are
  not (yet) included. diesel-async also supports MySQL natively, which this suite
  does not exercise.
- `insert_bulk_100` is not comparable for Diesel-async + SQLite: that suite is the
  only one that cannot issue a single multi-row statement, so it inserts 100 rows
  one statement at a time inside a transaction. Every other row of every table is
  like-for-like.
- The async suites here run one connection and `block_on` one operation at a time,
  which is the worst case for an async layer: nothing overlaps, so every
  `spawn_blocking` hop and executor wakeup lands directly in the measured latency.
  Under concurrency diesel-async and SQLx would recover much of the gap to the
  sync suites.
- The async suites reproduce to roughly ±10% between runs on this machine, so
  treat any single-digit difference between two async layers as a tie. Rebuilding
  the same benchmark against a different dependency set can shift the async
  numbers by that much on its own — see the SeaORM 2.0 section for how that was
  controlled for.
- SeaORM 2.0 requires Rust 1.94 or newer; the rest of the suite builds on older
  toolchains.

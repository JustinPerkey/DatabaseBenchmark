use anyhow::Result;

pub mod diesel_postgres;
pub mod diesel_schema;
pub mod diesel_sqlite;
pub mod rusqlite_sqlite;
pub mod rusqlite_sqlite_readonly;
pub mod seaorm_entities;
pub mod seaorm_postgres;
pub mod seaorm_sqlite;
pub mod sqlx_postgres;
pub mod sqlx_sqlite;
pub mod tokio_postgres_pg;

/// Number of rows inserted per bulk-insert iteration.
pub const BULK_SIZE: usize = 100;
/// Page size for the filtered list query and the join query.
pub const PAGE_SIZE: i64 = 50;
/// Users seeded with explicit ids 1..=SEED_USERS (targets for reads/updates).
pub const SEED_USERS: u32 = 1000;
/// Posts seeded referencing the seeded users.
pub const SEED_POSTS: u32 = 5000;
/// Delete targets are seeded with explicit ids DELETE_BASE+1.. so every
/// delete iteration removes a row that is guaranteed to exist.
pub const DELETE_BASE: u32 = 100_000;

pub const POSTGRES_URL: &str = "postgres://bench:bench@127.0.0.1:5432/bench";

/// The contract every database/ORM combination implements. One operation =
/// one timed unit of work. Suites run sequentially and own their schema.
pub trait Suite {
    fn name(&self) -> &'static str;
    /// True for suites that open the database read-only; the harness skips
    /// the write operations for them.
    fn read_only(&self) -> bool {
        false
    }
    /// Drop/create the schema and seed identical data via raw SQL.
    fn setup(&mut self, delete_rows: u32) -> Result<()>;
    /// Insert a single row.
    fn insert_one(&mut self, i: u32) -> Result<()>;
    /// Insert BULK_SIZE rows in one idiomatic batch statement.
    fn insert_bulk(&mut self, i: u32) -> Result<()>;
    /// Fetch one row by primary key, materialized into a typed struct.
    fn fetch_by_id(&mut self, i: u32) -> Result<()>;
    /// Fetch a filtered, ordered page of PAGE_SIZE rows.
    fn fetch_page(&mut self) -> Result<()>;
    /// Inner join posts->users with filters, PAGE_SIZE rows.
    fn join_query(&mut self) -> Result<()>;
    /// Update one column on one row by primary key.
    fn update_one(&mut self, i: u32) -> Result<()>;
    /// Delete one pre-seeded row by primary key.
    fn delete_one(&mut self, i: u32) -> Result<()>;
    fn teardown(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Plain row type used by the raw-driver suites (rusqlite / tokio-postgres).
#[allow(dead_code)]
#[derive(Debug)]
pub struct UserRow {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub active: bool,
    pub age: i32,
}

#[derive(Clone, Copy)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    fn bool_lit(self, b: bool) -> &'static str {
        match self {
            Dialect::Sqlite => {
                if b {
                    "1"
                } else {
                    "0"
                }
            }
            Dialect::Postgres => {
                if b {
                    "TRUE"
                } else {
                    "FALSE"
                }
            }
        }
    }
}

pub fn ddl(dialect: Dialect) -> String {
    let (pk, int) = match dialect {
        Dialect::Sqlite => ("INTEGER PRIMARY KEY", "INTEGER"),
        Dialect::Postgres => ("SERIAL PRIMARY KEY", "INT"),
    };
    format!(
        "DROP TABLE IF EXISTS posts;\n\
         DROP TABLE IF EXISTS users;\n\
         CREATE TABLE users (\n\
             id {pk},\n\
             name TEXT NOT NULL,\n\
             email TEXT NOT NULL,\n\
             active BOOLEAN NOT NULL,\n\
             age {int} NOT NULL\n\
         );\n\
         CREATE TABLE posts (\n\
             id {pk},\n\
             user_id {int} NOT NULL REFERENCES users(id),\n\
             title TEXT NOT NULL,\n\
             body TEXT NOT NULL,\n\
             published BOOLEAN NOT NULL\n\
         );\n\
         CREATE INDEX idx_posts_user_id ON posts(user_id);"
    )
}

/// Deterministic seed data, generated as raw multi-row INSERT statements so
/// every suite starts from byte-identical state regardless of its ORM.
pub fn seed_sql(dialect: Dialect, delete_rows: u32) -> Vec<String> {
    let mut stmts = Vec::new();
    let chunk = 500;

    // Users with explicit ids 1..=SEED_USERS.
    let mut rows: Vec<String> = Vec::new();
    for i in 1..=SEED_USERS {
        rows.push(format!(
            "({i},'user{i}','user{i}@example.com',{},{})",
            dialect.bool_lit(i % 4 != 0),
            20 + i % 50
        ));
    }
    // Delete targets with explicit ids above DELETE_BASE.
    for i in 1..=delete_rows {
        let id = DELETE_BASE + i;
        rows.push(format!(
            "({id},'victim{i}','victim{i}@example.com',{},30)",
            dialect.bool_lit(true)
        ));
    }
    for c in rows.chunks(chunk) {
        stmts.push(format!(
            "INSERT INTO users (id,name,email,active,age) VALUES {};",
            c.join(",")
        ));
    }

    // Posts referencing seeded users (auto ids).
    let mut rows: Vec<String> = Vec::new();
    for i in 1..=SEED_POSTS {
        rows.push(format!(
            "({},'post title {i}','lorem ipsum body text for post {i}',{})",
            1 + i % SEED_USERS,
            dialect.bool_lit(i % 3 != 0)
        ));
    }
    for c in rows.chunks(chunk) {
        stmts.push(format!(
            "INSERT INTO posts (user_id,title,body,published) VALUES {};",
            c.join(",")
        ));
    }

    if let Dialect::Postgres = dialect {
        // Explicit-id seeding bypasses the sequences; move them past every
        // seeded id so later default inserts cannot collide.
        stmts.push("SELECT setval('users_id_seq', 200000);".into());
        stmts.push("SELECT setval('posts_id_seq', 200000);".into());
    }
    stmts
}

/// Primary-key target for read/update ops: cycles through the seeded users.
pub fn target_id(i: u32) -> i32 {
    (1 + i % SEED_USERS) as i32
}

/// Primary-key target for delete ops: each iteration hits a fresh victim row.
pub fn delete_id(i: u32) -> i32 {
    (DELETE_BASE + 1 + i) as i32
}

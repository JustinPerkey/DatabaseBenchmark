use super::diesel_schema::{posts, users, NewUser, User};
use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

/// Diesel against a SQLite database loaded fully into memory. The file is
/// built and seeded on disk exactly like every other suite, then its contents
/// are copied into a `:memory:` connection before the first timed operation.
/// Afterwards every query touches RAM only — no filesystem, no fsync, no
/// journal files. The cost is durability: writes die with the process, so this
/// models caches, session stores, and read-mostly datasets rebuilt or
/// snapshotted elsewhere.
///
/// The load itself goes through `ATTACH` + `INSERT ... SELECT` rather than the
/// page-level backup API used by the rusqlite in-memory suite: Diesel exposes
/// no backup handle, and its `deserialize_readonly_database_from_buffer` would
/// give up writes. The one-time load cost is reported so the difference between
/// the two loading strategies stays visible.
pub struct DieselSqliteMemory {
    conn: Option<SqliteConnection>,
    path: PathBuf,
}

impl DieselSqliteMemory {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self { conn: None, path })
    }

    fn conn(&mut self) -> &mut SqliteConnection {
        self.conn.as_mut().expect("setup() opens the connection")
    }
}

impl Suite for DieselSqliteMemory {
    fn name(&self) -> &'static str {
        "Diesel (in-memory) + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        // Seed the on-disk file with a plain writable connection, exactly as a
        // deployed database would exist before being loaded into RAM.
        self.conn = None;
        let mut disk = SqliteConnection::establish(self.path.to_str().unwrap())?;
        disk.batch_execute(&super::ddl(Dialect::Sqlite))?;
        for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
            disk.batch_execute(&stmt)?;
        }
        drop(disk);
        let size = std::fs::metadata(&self.path)?.len();

        // Load disk -> memory. This is the one-time cost of "loading the
        // database in memory"; report it so the tradeoff is visible.
        let mut mem = SqliteConnection::establish(":memory:")?;
        let attach = self.path.to_str().unwrap().replace('\'', "''");
        let started = Instant::now();
        mem.batch_execute(&super::ddl(Dialect::Sqlite))?;
        mem.batch_execute(&format!(
            "ATTACH DATABASE '{attach}' AS src;\n\
             INSERT INTO main.users SELECT * FROM src.users;\n\
             INSERT INTO main.posts SELECT * FROM src.posts;\n\
             DETACH DATABASE src;"
        ))?;
        let load = started.elapsed();
        eprintln!(
            "    loaded {:.1} MiB from disk into memory in {:.1} ms",
            size as f64 / (1024.0 * 1024.0),
            load.as_secs_f64() * 1000.0
        );
        self.conn = Some(mem);
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        let row = NewUser {
            name: format!("new_user{i}"),
            email: format!("new_user{i}@example.com"),
            active: true,
            age: 30,
        };
        diesel::insert_into(users::table)
            .values(&row)
            .execute(self.conn())?;
        Ok(())
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        let rows: Vec<NewUser> = (0..BULK_SIZE)
            .map(|k| NewUser {
                name: format!("bulk_user{i}_{k}"),
                email: format!("bulk_user{i}_{k}@example.com"),
                active: true,
                age: 25,
            })
            .collect();
        let n = diesel::insert_into(users::table)
            .values(&rows)
            .execute(self.conn())?;
        ensure!(n == BULK_SIZE);
        Ok(())
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        let user: User = users::table
            .find(target_id(i))
            .select(User::as_select())
            .first(self.conn())?;
        ensure!(user.id == target_id(i));
        Ok(())
    }

    fn fetch_page(&mut self) -> Result<()> {
        let page: Vec<User> = users::table
            .filter(users::active.eq(true))
            .order(users::id.desc())
            .limit(PAGE_SIZE)
            .select(User::as_select())
            .load(self.conn())?;
        ensure!(page.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn join_query(&mut self) -> Result<()> {
        let rows: Vec<(String, String)> = posts::table
            .inner_join(users::table)
            .filter(users::active.eq(true))
            .filter(posts::published.eq(true))
            .select((posts::title, users::name))
            .limit(PAGE_SIZE)
            .load(self.conn())?;
        ensure!(rows.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        let name = format!("renamed{i}");
        let n = diesel::update(users::table.find(target_id(i)))
            .set(users::name.eq(name))
            .execute(self.conn())?;
        ensure!(n == 1);
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let n = diesel::delete(users::table.find(delete_id(i))).execute(self.conn())?;
        ensure!(n == 1);
        Ok(())
    }
}

use super::{delete_id, target_id, Dialect, Suite, UserRow, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use rusqlite::backup::Backup;
use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::time::Instant;

/// SQLite loaded fully into memory: the database file is built and seeded on
/// disk like every other suite, then copied page-by-page into a `:memory:`
/// connection with the SQLite backup API. Every operation afterwards touches
/// RAM only — no filesystem, no fsync, no journal files. The cost is
/// durability: writes die with the process, so this models caches, session
/// stores, and read-mostly datasets rebuilt or snapshotted elsewhere.
pub struct RusqliteSqliteMemory {
    conn: Option<Connection>,
    path: PathBuf,
}

impl RusqliteSqliteMemory {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self { conn: None, path })
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("setup() opens the connection")
    }
}

impl Suite for RusqliteSqliteMemory {
    fn name(&self) -> &'static str {
        "rusqlite (in-memory) + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        // Seed the on-disk file with a plain writable connection, exactly as
        // a deployed database would exist before being loaded into RAM.
        self.conn = None;
        let disk = Connection::open(&self.path)?;
        disk.execute_batch(&super::ddl(Dialect::Sqlite))?;
        for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
            disk.execute_batch(&stmt)?;
        }

        // Load disk -> memory. This is the one-time cost of "loading the
        // database in memory"; report it so the tradeoff is visible.
        let mut mem = Connection::open_in_memory()?;
        let started = Instant::now();
        Backup::new(&disk, &mut mem)?.run_to_completion(64, std::time::Duration::ZERO, None)?;
        let load = started.elapsed();
        let size: i64 = disk.query_row(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            [],
            |r| r.get(0),
        )?;
        disk.close().map_err(|(_, e)| e)?;
        eprintln!(
            "    loaded {:.1} MiB from disk into memory in {:.1} ms",
            size as f64 / (1024.0 * 1024.0),
            load.as_secs_f64() * 1000.0
        );
        self.conn = Some(mem);
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self.conn().prepare_cached(
            "INSERT INTO users (name, email, active, age) VALUES (?1, ?2, ?3, ?4)",
        )?;
        stmt.execute(params![
            format!("new_user{i}"),
            format!("new_user{i}@example.com"),
            true,
            30
        ])?;
        Ok(())
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        let conn = self.conn.as_mut().expect("setup() opens the connection");
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO users (name, email, active, age) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for k in 0..BULK_SIZE {
                stmt.execute(params![
                    format!("bulk_user{i}_{k}"),
                    format!("bulk_user{i}_{k}@example.com"),
                    true,
                    25
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn()
            .prepare_cached("SELECT id, name, email, active, age FROM users WHERE id = ?1")?;
        let user = stmt.query_row([target_id(i)], |row| {
            Ok(UserRow {
                id: row.get(0)?,
                name: row.get(1)?,
                email: row.get(2)?,
                active: row.get(3)?,
                age: row.get(4)?,
            })
        })?;
        ensure!(user.id == target_id(i));
        Ok(())
    }

    fn fetch_page(&mut self) -> Result<()> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT id, name, email, active, age FROM users \
             WHERE active = 1 ORDER BY id DESC LIMIT ?1",
        )?;
        let users: Vec<UserRow> = stmt
            .query_map([PAGE_SIZE], |row| {
                Ok(UserRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    email: row.get(2)?,
                    active: row.get(3)?,
                    age: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        ensure!(users.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn join_query(&mut self) -> Result<()> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT p.title, u.name FROM posts p \
             INNER JOIN users u ON u.id = p.user_id \
             WHERE u.active = 1 AND p.published = 1 LIMIT ?1",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map([PAGE_SIZE], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        ensure!(rows.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn()
            .prepare_cached("UPDATE users SET name = ?1 WHERE id = ?2")?;
        let n = stmt.execute(params![format!("renamed{i}"), target_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn()
            .prepare_cached("DELETE FROM users WHERE id = ?1")?;
        let n = stmt.execute([delete_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }
}

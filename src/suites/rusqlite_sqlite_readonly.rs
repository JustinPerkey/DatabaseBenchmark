use super::{target_id, Dialect, Suite, UserRow, PAGE_SIZE};
use anyhow::{bail, ensure, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::PathBuf;

/// SQLite opened read-only with `immutable=1` — the configuration for a
/// database deployed on a read-only filesystem (embedded Linux, squashfs).
/// Immutable mode skips all file locking and change detection, and never
/// touches journal/WAL files, so it works where the filesystem itself is
/// mounted read-only.
pub struct RusqliteSqliteReadonly {
    conn: Option<Connection>,
    path: PathBuf,
}

impl RusqliteSqliteReadonly {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self { conn: None, path })
    }

    fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("setup() opens the connection")
    }
}

impl Suite for RusqliteSqliteReadonly {
    fn name(&self) -> &'static str {
        "rusqlite (read-only) + SQLite"
    }

    fn read_only(&self) -> bool {
        true
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        // Build and seed with a plain writable connection (rollback journal,
        // not WAL — a WAL database can't be opened on a read-only mount),
        // then reopen the finished file read-only and immutable.
        self.conn = None;
        let writer = Connection::open(&self.path)?;
        writer.pragma_update(None, "journal_mode", "DELETE")?;
        writer.execute_batch(&super::ddl(Dialect::Sqlite))?;
        for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
            writer.execute_batch(&stmt)?;
        }
        writer.close().map_err(|(_, e)| e)?;

        let uri = format!("file:{}?immutable=1", self.path.display());
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "mmap_size", 256 * 1024 * 1024)?;
        self.conn = Some(conn);
        Ok(())
    }

    fn insert_one(&mut self, _i: u32) -> Result<()> {
        bail!("read-only suite");
    }

    fn insert_bulk(&mut self, _i: u32) -> Result<()> {
        bail!("read-only suite");
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

    fn update_one(&mut self, _i: u32) -> Result<()> {
        bail!("read-only suite");
    }

    fn delete_one(&mut self, _i: u32) -> Result<()> {
        bail!("read-only suite");
    }
}

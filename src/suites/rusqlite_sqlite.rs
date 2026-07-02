use super::{delete_id, target_id, Dialect, Suite, UserRow, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use rusqlite::{params, Connection};
use std::path::PathBuf;

pub struct RusqliteSqlite {
    conn: Connection,
    _path: PathBuf,
}

impl RusqliteSqlite {
    pub fn new(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Ok(Self { conn, _path: path })
    }
}

impl Suite for RusqliteSqlite {
    fn name(&self) -> &'static str {
        "rusqlite (raw) + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.conn.execute_batch(&super::ddl(Dialect::Sqlite))?;
        for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
            self.conn.execute_batch(&stmt)?;
        }
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
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
        let tx = self.conn.transaction()?;
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
            .conn
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
        let mut stmt = self.conn.prepare_cached(
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
        let mut stmt = self.conn.prepare_cached(
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
            .conn
            .prepare_cached("UPDATE users SET name = ?1 WHERE id = ?2")?;
        let n = stmt.execute(params![format!("renamed{i}"), target_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare_cached("DELETE FROM users WHERE id = ?1")?;
        let n = stmt.execute([delete_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }
}

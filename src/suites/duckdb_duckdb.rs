use super::{delete_id, target_id, Dialect, Suite, UserRow, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use duckdb::{params, Connection};
use std::path::PathBuf;

pub struct DuckDb {
    conn: Connection,
    /// DuckDB's appender needs explicit ids; hand them out from a range far
    /// above everything else in the table.
    next_bulk_id: i32,
}

impl DuckDb {
    pub fn new(path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&path)?;
        Ok(Self {
            conn,
            next_bulk_id: 500_000,
        })
    }
}

impl Suite for DuckDb {
    fn name(&self) -> &'static str {
        "duckdb-rs + DuckDB"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.conn.execute_batch(&super::ddl(Dialect::DuckDb))?;
        for stmt in super::seed_sql(Dialect::DuckDb, delete_rows) {
            self.conn.execute_batch(&stmt)?;
        }
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare_cached("INSERT INTO users (name, email, active, age) VALUES (?, ?, ?, ?)")?;
        stmt.execute(params![
            format!("new_user{i}"),
            format!("new_user{i}@example.com"),
            true,
            30
        ])?;
        Ok(())
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        let mut appender = self.conn.appender("users")?;
        for k in 0..BULK_SIZE {
            appender.append_row(params![
                self.next_bulk_id + k as i32,
                format!("bulk_user{i}_{k}"),
                format!("bulk_user{i}_{k}@example.com"),
                true,
                25
            ])?;
        }
        appender.flush()?;
        self.next_bulk_id += BULK_SIZE as i32;
        Ok(())
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, name, email, active, age FROM users WHERE id = ?")?;
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
             WHERE active ORDER BY id DESC LIMIT ?",
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
            .collect::<duckdb::Result<_>>()?;
        ensure!(users.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn join_query(&mut self) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT p.title, u.name FROM posts p \
             INNER JOIN users u ON u.id = p.user_id \
             WHERE u.active AND p.published LIMIT ?",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map([PAGE_SIZE], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<duckdb::Result<_>>()?;
        ensure!(rows.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare_cached("UPDATE users SET name = ? WHERE id = ?")?;
        let n = stmt.execute(params![format!("renamed{i}"), target_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let mut stmt = self.conn.prepare_cached("DELETE FROM users WHERE id = ?")?;
        let n = stmt.execute([delete_id(i)])?;
        ensure!(n == 1);
        Ok(())
    }
}

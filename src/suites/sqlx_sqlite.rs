use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::QueryBuilder;
use std::path::PathBuf;
use tokio::runtime::Runtime;

#[derive(sqlx::FromRow)]
#[allow(dead_code)]
struct User {
    id: i32,
    name: String,
    email: String,
    active: bool,
    age: i32,
}

pub struct SqlxSqlite {
    rt: Runtime,
    pool: SqlitePool,
}

impl SqlxSqlite {
    pub fn new(path: PathBuf) -> Result<Self> {
        let rt = Runtime::new()?;
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
        let pool = rt.block_on(
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts),
        )?;
        Ok(Self { rt, pool })
    }
}

impl Suite for SqlxSqlite {
    fn name(&self) -> &'static str {
        "SQLx + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.rt.block_on(async {
            sqlx::raw_sql(&super::ddl(Dialect::Sqlite))
                .execute(&self.pool)
                .await?;
            for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
                sqlx::raw_sql(&stmt).execute(&self.pool).await?;
            }
            Ok(())
        })
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            sqlx::query("INSERT INTO users (name, email, active, age) VALUES (?, ?, ?, ?)")
                .bind(format!("new_user{i}"))
                .bind(format!("new_user{i}@example.com"))
                .bind(true)
                .bind(30)
                .execute(&self.pool)
                .await?;
            Ok(())
        })
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let mut qb = QueryBuilder::new("INSERT INTO users (name, email, active, age) ");
            qb.push_values(0..BULK_SIZE, |mut b, k| {
                b.push_bind(format!("bulk_user{i}_{k}"))
                    .push_bind(format!("bulk_user{i}_{k}@example.com"))
                    .push_bind(true)
                    .push_bind(25);
            });
            let res = qb.build().execute(&self.pool).await?;
            ensure!(res.rows_affected() == BULK_SIZE as u64);
            Ok(())
        })
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let user: User =
                sqlx::query_as("SELECT id, name, email, active, age FROM users WHERE id = ?")
                    .bind(target_id(i))
                    .fetch_one(&self.pool)
                    .await?;
            ensure!(user.id == target_id(i));
            Ok(())
        })
    }

    fn fetch_page(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let users: Vec<User> = sqlx::query_as(
                "SELECT id, name, email, active, age FROM users \
                 WHERE active = 1 ORDER BY id DESC LIMIT ?",
            )
            .bind(PAGE_SIZE)
            .fetch_all(&self.pool)
            .await?;
            ensure!(users.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn join_query(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT p.title, u.name FROM posts p \
                 INNER JOIN users u ON u.id = p.user_id \
                 WHERE u.active = 1 AND p.published = 1 LIMIT ?",
            )
            .bind(PAGE_SIZE)
            .fetch_all(&self.pool)
            .await?;
            ensure!(rows.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let res = sqlx::query("UPDATE users SET name = ? WHERE id = ?")
                .bind(format!("renamed{i}"))
                .bind(target_id(i))
                .execute(&self.pool)
                .await?;
            ensure!(res.rows_affected() == 1);
            Ok(())
        })
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let res = sqlx::query("DELETE FROM users WHERE id = ?")
                .bind(delete_id(i))
                .execute(&self.pool)
                .await?;
            ensure!(res.rows_affected() == 1);
            Ok(())
        })
    }

    fn teardown(&mut self) -> Result<()> {
        self.rt.block_on(self.pool.close());
        Ok(())
    }
}

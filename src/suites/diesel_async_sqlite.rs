use super::diesel_schema::{posts, users, NewUser, User};
use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_async::sync_connection_wrapper::SyncConnectionWrapper;
use diesel_async::{AsyncConnection, RunQueryDsl, SimpleAsyncConnection};
use std::path::PathBuf;
use tokio::runtime::Runtime;

/// SQLite has no async wire protocol, so diesel-async wraps the ordinary sync
/// `SqliteConnection` and moves every statement onto `tokio::spawn_blocking`.
/// The query code is identical to the sync `diesel_sqlite` suite, so the delta
/// between the two suites is exactly the cost of that hop.
pub struct DieselAsyncSqlite {
    rt: Runtime,
    conn: SyncConnectionWrapper<SqliteConnection>,
}

impl DieselAsyncSqlite {
    pub fn new(path: PathBuf) -> Result<Self> {
        let rt = Runtime::new()?;
        let mut conn = rt.block_on(SyncConnectionWrapper::<SqliteConnection>::establish(
            path.to_str().unwrap(),
        ))?;
        rt.block_on(conn.batch_execute("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;"))?;
        Ok(Self { rt, conn })
    }
}

impl Suite for DieselAsyncSqlite {
    fn name(&self) -> &'static str {
        "Diesel-async + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.rt.block_on(async {
            self.conn
                .batch_execute(&super::ddl(Dialect::Sqlite))
                .await?;
            for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
                self.conn.batch_execute(&stmt).await?;
            }
            Ok(())
        })
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            diesel::insert_into(users::table)
                .values(&NewUser {
                    name: format!("new_user{i}"),
                    email: format!("new_user{i}@example.com"),
                    active: true,
                    age: 30,
                })
                .execute(&mut self.conn)
                .await?;
            Ok(())
        })
    }

    /// diesel-async cannot express Diesel's multi-row SQLite insert: the
    /// single-statement `VALUES (..),(..)` form is produced by a
    /// `SqliteConnection`-specialized `ExecuteDsl` impl in diesel itself,
    /// which diesel-async does not have, so `.values(&vec)` does not compile
    /// against `SyncConnectionWrapper`. The idiomatic fallback is one
    /// statement per row inside a single transaction — so this row is *not*
    /// comparable to the other suites' one-statement bulk insert.
    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let rows: Vec<NewUser> = (0..BULK_SIZE)
                .map(|k| NewUser {
                    name: format!("bulk_user{i}_{k}"),
                    email: format!("bulk_user{i}_{k}@example.com"),
                    active: true,
                    age: 25,
                })
                .collect();
            self.conn.batch_execute("BEGIN").await?;
            let mut n = 0;
            for row in &rows {
                n += diesel::insert_into(users::table)
                    .values(row)
                    .execute(&mut self.conn)
                    .await?;
            }
            self.conn.batch_execute("COMMIT").await?;
            ensure!(n == BULK_SIZE);
            Ok(())
        })
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let user: User = users::table
                .find(target_id(i))
                .select(User::as_select())
                .first(&mut self.conn)
                .await?;
            ensure!(user.id == target_id(i));
            Ok(())
        })
    }

    fn fetch_page(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let page: Vec<User> = users::table
                .filter(users::active.eq(true))
                .order(users::id.desc())
                .limit(PAGE_SIZE)
                .select(User::as_select())
                .load(&mut self.conn)
                .await?;
            ensure!(page.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn join_query(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let rows: Vec<(String, String)> = posts::table
                .inner_join(users::table)
                .filter(users::active.eq(true))
                .filter(posts::published.eq(true))
                .select((posts::title, users::name))
                .limit(PAGE_SIZE)
                .load(&mut self.conn)
                .await?;
            ensure!(rows.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let n = diesel::update(users::table.find(target_id(i)))
                .set(users::name.eq(format!("renamed{i}")))
                .execute(&mut self.conn)
                .await?;
            ensure!(n == 1);
            Ok(())
        })
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let n = diesel::delete(users::table.find(delete_id(i)))
                .execute(&mut self.conn)
                .await?;
            ensure!(n == 1);
            Ok(())
        })
    }
}

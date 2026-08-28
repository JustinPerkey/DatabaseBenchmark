use super::diesel_schema::{posts, users, NewUser, User};
use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE, POSTGRES_URL};
use anyhow::{ensure, Result};
use diesel::prelude::*;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};
use tokio::runtime::Runtime;

/// Diesel's query DSL over an async connection: identical query code to the
/// sync `diesel_postgres` suite, but the connection speaks the Postgres wire
/// protocol through tokio-postgres instead of libpq.
pub struct DieselAsyncPostgres {
    rt: Runtime,
    conn: AsyncPgConnection,
}

impl DieselAsyncPostgres {
    pub fn new() -> Result<Self> {
        let rt = Runtime::new()?;
        let conn = rt.block_on(AsyncPgConnection::establish(POSTGRES_URL))?;
        Ok(Self { rt, conn })
    }
}

impl Suite for DieselAsyncPostgres {
    fn name(&self) -> &'static str {
        "Diesel-async + PostgreSQL"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.rt.block_on(async {
            self.conn
                .batch_execute(&super::ddl(Dialect::Postgres))
                .await?;
            for stmt in super::seed_sql(Dialect::Postgres, delete_rows) {
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
            let n = diesel::insert_into(users::table)
                .values(&rows)
                .execute(&mut self.conn)
                .await?;
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

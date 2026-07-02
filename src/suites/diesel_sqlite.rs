use super::diesel_schema::{posts, users, NewUser, User};
use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Result};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use std::path::PathBuf;

pub struct DieselSqlite {
    conn: SqliteConnection,
}

impl DieselSqlite {
    pub fn new(path: PathBuf) -> Result<Self> {
        let mut conn = SqliteConnection::establish(path.to_str().unwrap())?;
        conn.batch_execute("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Ok(Self { conn })
    }
}

impl Suite for DieselSqlite {
    fn name(&self) -> &'static str {
        "Diesel + SQLite"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.conn.batch_execute(&super::ddl(Dialect::Sqlite))?;
        for stmt in super::seed_sql(Dialect::Sqlite, delete_rows) {
            self.conn.batch_execute(&stmt)?;
        }
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        diesel::insert_into(users::table)
            .values(&NewUser {
                name: format!("new_user{i}"),
                email: format!("new_user{i}@example.com"),
                active: true,
                age: 30,
            })
            .execute(&mut self.conn)?;
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
            .execute(&mut self.conn)?;
        ensure!(n == BULK_SIZE);
        Ok(())
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        let user: User = users::table
            .find(target_id(i))
            .select(User::as_select())
            .first(&mut self.conn)?;
        ensure!(user.id == target_id(i));
        Ok(())
    }

    fn fetch_page(&mut self) -> Result<()> {
        let page: Vec<User> = users::table
            .filter(users::active.eq(true))
            .order(users::id.desc())
            .limit(PAGE_SIZE)
            .select(User::as_select())
            .load(&mut self.conn)?;
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
            .load(&mut self.conn)?;
        ensure!(rows.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        let n = diesel::update(users::table.find(target_id(i)))
            .set(users::name.eq(format!("renamed{i}")))
            .execute(&mut self.conn)?;
        ensure!(n == 1);
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let n = diesel::delete(users::table.find(delete_id(i))).execute(&mut self.conn)?;
        ensure!(n == 1);
        Ok(())
    }
}

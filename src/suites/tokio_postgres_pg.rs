use super::{delete_id, target_id, Dialect, Suite, UserRow, BULK_SIZE, PAGE_SIZE, POSTGRES_URL};
use anyhow::{ensure, Result};
use tokio::runtime::Runtime;
use tokio_postgres::{Client, NoTls, Statement};

pub struct TokioPostgres {
    rt: Runtime,
    client: Client,
    stmts: Option<Stmts>,
}

struct Stmts {
    insert_one: Statement,
    insert_bulk: Statement,
    fetch_by_id: Statement,
    fetch_page: Statement,
    join_query: Statement,
    update_one: Statement,
    delete_one: Statement,
}

impl TokioPostgres {
    pub fn new() -> Result<Self> {
        let rt = Runtime::new()?;
        let client = rt.block_on(async {
            let (client, connection) = tokio_postgres::connect(POSTGRES_URL, NoTls).await?;
            // Drive the connection on a background task for the suite's lifetime.
            tokio::spawn(async move {
                let _ = connection.await;
            });
            Ok::<_, anyhow::Error>(client)
        })?;
        Ok(Self {
            rt,
            client,
            stmts: None,
        })
    }

    fn stmts(&self) -> &Stmts {
        self.stmts.as_ref().expect("setup() prepares statements")
    }
}

impl Suite for TokioPostgres {
    fn name(&self) -> &'static str {
        "tokio-postgres (raw) + PostgreSQL"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.rt.block_on(async {
            self.client
                .batch_execute(&super::ddl(Dialect::Postgres))
                .await?;
            for stmt in super::seed_sql(Dialect::Postgres, delete_rows) {
                self.client.batch_execute(&stmt).await?;
            }
            self.stmts = Some(Stmts {
                insert_one: self
                    .client
                    .prepare("INSERT INTO users (name, email, active, age) VALUES ($1, $2, $3, $4)")
                    .await?,
                insert_bulk: self
                    .client
                    .prepare(
                        "INSERT INTO users (name, email, active, age) \
                         SELECT * FROM UNNEST($1::text[], $2::text[], $3::bool[], $4::int4[])",
                    )
                    .await?,
                fetch_by_id: self
                    .client
                    .prepare("SELECT id, name, email, active, age FROM users WHERE id = $1")
                    .await?,
                fetch_page: self
                    .client
                    .prepare(
                        "SELECT id, name, email, active, age FROM users \
                         WHERE active ORDER BY id DESC LIMIT $1",
                    )
                    .await?,
                join_query: self
                    .client
                    .prepare(
                        "SELECT p.title, u.name FROM posts p \
                         INNER JOIN users u ON u.id = p.user_id \
                         WHERE u.active AND p.published LIMIT $1",
                    )
                    .await?,
                update_one: self
                    .client
                    .prepare("UPDATE users SET name = $1 WHERE id = $2")
                    .await?,
                delete_one: self
                    .client
                    .prepare("DELETE FROM users WHERE id = $1")
                    .await?,
            });
            Ok(())
        })
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            self.client
                .execute(
                    &self.stmts().insert_one,
                    &[
                        &format!("new_user{i}"),
                        &format!("new_user{i}@example.com"),
                        &true,
                        &30i32,
                    ],
                )
                .await?;
            Ok(())
        })
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        let names: Vec<String> = (0..BULK_SIZE)
            .map(|k| format!("bulk_user{i}_{k}"))
            .collect();
        let emails: Vec<String> = (0..BULK_SIZE)
            .map(|k| format!("bulk_user{i}_{k}@example.com"))
            .collect();
        let actives = vec![true; BULK_SIZE];
        let ages = vec![25i32; BULK_SIZE];
        self.rt.block_on(async {
            let n = self
                .client
                .execute(
                    &self.stmts().insert_bulk,
                    &[&names, &emails, &actives, &ages],
                )
                .await?;
            ensure!(n == BULK_SIZE as u64);
            Ok(())
        })
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let row = self
                .client
                .query_one(&self.stmts().fetch_by_id, &[&target_id(i)])
                .await?;
            let user = UserRow {
                id: row.get(0),
                name: row.get(1),
                email: row.get(2),
                active: row.get(3),
                age: row.get(4),
            };
            ensure!(user.id == target_id(i));
            Ok(())
        })
    }

    fn fetch_page(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let rows = self
                .client
                .query(&self.stmts().fetch_page, &[&PAGE_SIZE])
                .await?;
            let users: Vec<UserRow> = rows
                .iter()
                .map(|row| UserRow {
                    id: row.get(0),
                    name: row.get(1),
                    email: row.get(2),
                    active: row.get(3),
                    age: row.get(4),
                })
                .collect();
            ensure!(users.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn join_query(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let rows = self
                .client
                .query(&self.stmts().join_query, &[&PAGE_SIZE])
                .await?;
            let pairs: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
            ensure!(pairs.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let n = self
                .client
                .execute(
                    &self.stmts().update_one,
                    &[&format!("renamed{i}"), &target_id(i)],
                )
                .await?;
            ensure!(n == 1);
            Ok(())
        })
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let n = self
                .client
                .execute(&self.stmts().delete_one, &[&delete_id(i)])
                .await?;
            ensure!(n == 1);
            Ok(())
        })
    }
}

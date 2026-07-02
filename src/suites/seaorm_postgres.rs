use super::seaorm_entities::{post, user};
use super::{delete_id, target_id, Dialect, Suite, BULK_SIZE, PAGE_SIZE, POSTGRES_URL};
use anyhow::{ensure, Result};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, Set,
};
use tokio::runtime::Runtime;

pub struct SeaOrmPostgres {
    rt: Runtime,
    db: DatabaseConnection,
}

impl SeaOrmPostgres {
    pub fn new() -> Result<Self> {
        let rt = Runtime::new()?;
        let mut opts = ConnectOptions::new(POSTGRES_URL);
        opts.max_connections(1).sqlx_logging(false);
        let db = rt.block_on(Database::connect(opts))?;
        Ok(Self { rt, db })
    }
}

impl Suite for SeaOrmPostgres {
    fn name(&self) -> &'static str {
        "SeaORM + PostgreSQL"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        self.rt.block_on(async {
            self.db
                .execute_unprepared(&super::ddl(Dialect::Postgres))
                .await?;
            for stmt in super::seed_sql(Dialect::Postgres, delete_rows) {
                self.db.execute_unprepared(&stmt).await?;
            }
            Ok(())
        })
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let am = user::ActiveModel {
                name: Set(format!("new_user{i}")),
                email: Set(format!("new_user{i}@example.com")),
                active: Set(true),
                age: Set(30),
                ..Default::default()
            };
            user::Entity::insert(am).exec(&self.db).await?;
            Ok(())
        })
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let rows = (0..BULK_SIZE).map(|k| user::ActiveModel {
                name: Set(format!("bulk_user{i}_{k}")),
                email: Set(format!("bulk_user{i}_{k}@example.com")),
                active: Set(true),
                age: Set(25),
                ..Default::default()
            });
            user::Entity::insert_many(rows).exec(&self.db).await?;
            Ok(())
        })
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let found = user::Entity::find_by_id(target_id(i)).one(&self.db).await?;
            ensure!(found.is_some_and(|u| u.id == target_id(i)));
            Ok(())
        })
    }

    fn fetch_page(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let page: Vec<user::Model> = user::Entity::find()
                .filter(user::Column::Active.eq(true))
                .order_by_desc(user::Column::Id)
                .limit(PAGE_SIZE as u64)
                .all(&self.db)
                .await?;
            ensure!(page.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn join_query(&mut self) -> Result<()> {
        self.rt.block_on(async {
            let rows: Vec<(post::Model, Option<user::Model>)> = post::Entity::find()
                .find_also_related(user::Entity)
                .filter(user::Column::Active.eq(true))
                .filter(post::Column::Published.eq(true))
                .limit(PAGE_SIZE as u64)
                .all(&self.db)
                .await?;
            ensure!(rows.len() == PAGE_SIZE as usize);
            Ok(())
        })
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let res = user::Entity::update_many()
                .col_expr(user::Column::Name, Expr::value(format!("renamed{i}")))
                .filter(user::Column::Id.eq(target_id(i)))
                .exec(&self.db)
                .await?;
            ensure!(res.rows_affected == 1);
            Ok(())
        })
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        self.rt.block_on(async {
            let res = user::Entity::delete_by_id(delete_id(i))
                .exec(&self.db)
                .await?;
            ensure!(res.rows_affected == 1);
            Ok(())
        })
    }
}

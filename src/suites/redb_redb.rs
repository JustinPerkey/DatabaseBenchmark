use super::{delete_id, target_id, PostRow, Suite, UserRow, BULK_SIZE, PAGE_SIZE};
use anyhow::{ensure, Context, Result};
use redb::{Database, Durability, ReadableTable, TableDefinition, WriteTransaction};
use std::path::PathBuf;

// redb is a key-value store: no SQL, no secondary indexes, no query planner.
// Rows are bincode-serialized structs keyed by id; everything a SQL WHERE /
// ORDER BY / JOIN would do happens in application code below.
const USERS: TableDefinition<u32, &[u8]> = TableDefinition::new("users");
const POSTS: TableDefinition<u32, &[u8]> = TableDefinition::new("posts");

pub struct Redb {
    db: Database,
    next_id: u32,
}

impl Redb {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self {
            db: Database::create(path)?,
            next_id: 500_000,
        })
    }

    /// redb fsyncs every commit by default; the SQLite suites run WAL +
    /// synchronous=NORMAL (no per-commit fsync), so match that durability
    /// level to keep write timings comparable.
    fn begin_write(&self) -> Result<WriteTransaction> {
        let mut txn = self.db.begin_write()?;
        txn.set_durability(Durability::Eventual);
        Ok(txn)
    }
}

impl Suite for Redb {
    fn name(&self) -> &'static str {
        "redb (embedded KV)"
    }

    fn setup(&mut self, delete_rows: u32) -> Result<()> {
        let txn = self.begin_write()?;
        {
            let mut users = txn.open_table(USERS)?;
            for u in super::seed_users(delete_rows) {
                users.insert(u.id as u32, bincode::serialize(&u)?.as_slice())?;
            }
            let mut posts = txn.open_table(POSTS)?;
            for (id, p) in super::seed_posts().iter().enumerate() {
                posts.insert(1 + id as u32, bincode::serialize(p)?.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    fn insert_one(&mut self, i: u32) -> Result<()> {
        let user = UserRow {
            id: self.next_id as i32,
            name: format!("new_user{i}"),
            email: format!("new_user{i}@example.com"),
            active: true,
            age: 30,
        };
        let txn = self.begin_write()?;
        {
            let mut users = txn.open_table(USERS)?;
            users.insert(self.next_id, bincode::serialize(&user)?.as_slice())?;
        }
        txn.commit()?;
        self.next_id += 1;
        Ok(())
    }

    fn insert_bulk(&mut self, i: u32) -> Result<()> {
        let txn = self.begin_write()?;
        {
            let mut users = txn.open_table(USERS)?;
            for k in 0..BULK_SIZE {
                let id = self.next_id + k as u32;
                let user = UserRow {
                    id: id as i32,
                    name: format!("bulk_user{i}_{k}"),
                    email: format!("bulk_user{i}_{k}@example.com"),
                    active: true,
                    age: 25,
                };
                users.insert(id, bincode::serialize(&user)?.as_slice())?;
            }
        }
        txn.commit()?;
        self.next_id += BULK_SIZE as u32;
        Ok(())
    }

    fn fetch_by_id(&mut self, i: u32) -> Result<()> {
        let txn = self.db.begin_read()?;
        let users = txn.open_table(USERS)?;
        let bytes = users.get(target_id(i) as u32)?.context("user not found")?;
        let user: UserRow = bincode::deserialize(bytes.value())?;
        ensure!(user.id == target_id(i));
        Ok(())
    }

    fn fetch_page(&mut self) -> Result<()> {
        // WHERE active ORDER BY id DESC LIMIT 50, by hand: keys iterate in
        // order, so walk them backwards and filter.
        let txn = self.db.begin_read()?;
        let users = txn.open_table(USERS)?;
        let mut page: Vec<UserRow> = Vec::with_capacity(PAGE_SIZE as usize);
        for entry in users.iter()?.rev() {
            let (_, bytes) = entry?;
            let user: UserRow = bincode::deserialize(bytes.value())?;
            if user.active {
                page.push(user);
                if page.len() == PAGE_SIZE as usize {
                    break;
                }
            }
        }
        ensure!(page.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn join_query(&mut self) -> Result<()> {
        // posts JOIN users, by hand: scan posts, point-lookup each user.
        let txn = self.db.begin_read()?;
        let posts = txn.open_table(POSTS)?;
        let users = txn.open_table(USERS)?;
        let mut rows: Vec<(String, String)> = Vec::with_capacity(PAGE_SIZE as usize);
        for entry in posts.iter()? {
            let (_, bytes) = entry?;
            let post: PostRow = bincode::deserialize(bytes.value())?;
            if !post.published {
                continue;
            }
            let user_bytes = users.get(post.user_id as u32)?.context("user not found")?;
            let user: UserRow = bincode::deserialize(user_bytes.value())?;
            if user.active {
                rows.push((post.title, user.name));
                if rows.len() == PAGE_SIZE as usize {
                    break;
                }
            }
        }
        ensure!(rows.len() == PAGE_SIZE as usize);
        Ok(())
    }

    fn update_one(&mut self, i: u32) -> Result<()> {
        let txn = self.begin_write()?;
        {
            let mut users = txn.open_table(USERS)?;
            let mut user: UserRow = {
                let bytes = users.get(target_id(i) as u32)?.context("user not found")?;
                bincode::deserialize(bytes.value())?
            };
            user.name = format!("renamed{i}");
            users.insert(target_id(i) as u32, bincode::serialize(&user)?.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    fn delete_one(&mut self, i: u32) -> Result<()> {
        let txn = self.begin_write()?;
        let existed;
        {
            let mut users = txn.open_table(USERS)?;
            let removed = users.remove(delete_id(i) as u32)?;
            existed = removed.is_some();
        }
        txn.commit()?;
        ensure!(existed);
        Ok(())
    }
}

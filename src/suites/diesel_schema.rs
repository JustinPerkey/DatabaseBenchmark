use diesel::prelude::*;

diesel::table! {
    users (id) {
        id -> Integer,
        name -> Text,
        email -> Text,
        active -> Bool,
        age -> Integer,
    }
}

diesel::table! {
    posts (id) {
        id -> Integer,
        user_id -> Integer,
        title -> Text,
        body -> Text,
        published -> Bool,
    }
}

diesel::joinable!(posts -> users (user_id));
diesel::allow_tables_to_appear_in_same_query!(users, posts);

#[derive(Queryable, Selectable, Debug)]
#[diesel(table_name = users)]
#[allow(dead_code)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub active: bool,
    pub age: i32,
}

#[derive(Insertable)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub active: bool,
    pub age: i32,
}

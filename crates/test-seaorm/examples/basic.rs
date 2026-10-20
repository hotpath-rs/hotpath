//! Demonstrates `hotpath::sqlx_tracing_layer()` capturing SeaORM queries. The
//! SeaORM sqlx drivers execute every statement through a sqlx pool, so sqlx's
//! own `sqlx::query` completion events reach the layer - no SeaORM-specific
//! front-end is involved.
//!
//! Set `SEAORM_SQLX_LOGGING=0` to connect with `sqlx_logging(false)`: SeaORM
//! then disables sqlx statement logging, the events are never emitted, and the
//! SQL report stays empty.
//!
//! Run with:
//!   cargo run -p test-seaorm --example basic --features hotpath

use hotpath::{HotpathGuardBuilder, Section};
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend,
    EntityTrait, PaginatorTrait, QueryFilter, Set, Statement, TransactionTrait,
};
use tracing_subscriber::prelude::*;

mod user {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub name: String,
        pub age: i32,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

fn new_user(name: String, age: i32) -> user::ActiveModel {
    user::ActiveModel {
        name: Set(name),
        age: Set(age),
        ..Default::default()
    }
}

async fn connect() -> Result<DatabaseConnection, sea_orm::DbErr> {
    let sqlx_logging = std::env::var("SEAORM_SQLX_LOGGING").as_deref() != Ok("0");
    let mut opts = ConnectOptions::new("sqlite::memory:");
    // Every in-memory sqlite connection is its own database.
    opts.max_connections(1).sqlx_logging(sqlx_logging);
    Database::connect(opts).await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(hotpath::sqlx_tracing_layer())
        .init();

    let _guard = HotpathGuardBuilder::new("main")
        .sections(vec![Section::Sql])
        .build();

    let db = connect().await?;

    db.execute_unprepared("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)")
        .await?;

    // 50 entity inserts, identical generated text -> one bucket.
    for i in 0..50 {
        user::Entity::insert(new_user(format!("user{i}"), 20 + i))
            .exec(&db)
            .await?;
    }

    // 30 point lookups, bind params -> one bucket.
    for i in 1..=30 {
        let _ = user::Entity::find_by_id(i).one(&db).await?;
    }

    // 20 filtered selects, bind params -> one bucket.
    for i in 1..=20 {
        let _ = user::Entity::find()
            .filter(user::Column::Age.eq(20 + i))
            .all(&db)
            .await?;
    }

    // 10 raw statements with VARYING inline literals -> normalization merges them.
    for i in 1..=10 {
        let sql = format!("SELECT name FROM users WHERE age = {}", 20 + i);
        let _ = db
            .query_all_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await?;
    }

    // 5 aggregates -> one bucket.
    for _ in 0..5 {
        let _ = user::Entity::find().count(&db).await?;
    }

    // Transaction-internal queries are captured too.
    let txn = db.begin().await?;
    user::Entity::insert(new_user("in_tx".to_string(), 99))
        .exec(&txn)
        .await?;
    txn.commit().await?;

    println!("SeaORM example completed!");

    Ok(())
}

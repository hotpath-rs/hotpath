//! Integration tests for SeaORM query profiling through the `sqlx` tracing
//! layer.
//!
//! Runs the `test-seaorm` `basic` example as a subprocess and asserts on its
//! report. SeaORM's sqlx drivers execute every statement through a sqlx pool,
//! so `sqlx_tracing_layer()` captures them. This also pins SeaORM's generated
//! SQL and its default-on sqlx statement logging - a SeaORM upgrade that
//! changes either would surface here.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use crate::common::example::Example;

    fn basic() -> Example {
        Example::new("test-seaorm", "basic").json()
    }

    #[test]
    fn test_seaorm_queries_captured() {
        let report = basic().report();
        let sql = report.sql.expect("No sql section in report");

        let count = |query: &str| {
            sql.data
                .iter()
                .find(|e| e.query == query)
                .unwrap_or_else(|| panic!("{query:?} missing from {:#?}", sql.data))
                .count
        };

        // 50 loop inserts + 1 transaction-internal insert.
        assert_eq!(
            count(r#"INSERT INTO "users" ("name", "age") VALUES (?, ?)"#),
            51
        );
        assert_eq!(
            count(
                r#"SELECT "users"."id", "users"."name", "users"."age" FROM "users" WHERE "users"."id" = ? LIMIT ?"#
            ),
            30
        );
        assert_eq!(
            count(
                r#"SELECT "users"."id", "users"."name", "users"."age" FROM "users" WHERE "users"."age" = ?"#
            ),
            20
        );
        // Raw statements with varying inline literals merge into one bucket.
        assert_eq!(count("SELECT name FROM users WHERE age = ?"), 10);
        assert_eq!(
            count(
                r#"SELECT COUNT(*) AS num_items FROM (SELECT "users"."id", "users"."name", "users"."age" FROM "users") AS "sub_query""#
            ),
            5
        );
    }

    // `ConnectOptions::sqlx_logging(false)` turns sqlx statement logging off,
    // so no `sqlx::query` events reach the layer and nothing is recorded.
    #[test]
    fn test_seaorm_sqlx_logging_disabled() {
        let report = basic().env("SEAORM_SQLX_LOGGING", "0").report();
        let recorded = report.sql.map_or(0, |sql| sql.total_calls);
        assert_eq!(recorded, 0, "expected no SQL entries with sqlx_logging off");
    }
}

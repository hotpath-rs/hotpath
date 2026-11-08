//! Integration tests for the `sqlx` tracing-layer front-end (sqlite backend).
//!
//! These run the `test-sqlx-08` and `test-sqlx-09` `basic` examples as
//! subprocesses and assert on their reports. The same hotpath layer feeds both:
//! the `sqlx::query` event field schema (`db.statement` / `summary` /
//! `elapsed_secs`) is identical across sqlx 0.8 and 0.9, so a single layer
//! covers both. These tests also pin that schema - a sqlx upgrade that renames
//! or drops those fields would empty the SQL report and fail here.
//!
//! PostgreSQL coverage lives in `sql_pg.rs`.
#[path = "common/support.rs"]
mod common;

#[cfg(all(test, feature = "hotpath"))]
pub mod tests {
    use std::time::Duration;

    use hotpath::json::{JsonSqlList, JsonSqlLogsList};

    use crate::common::example::{poll_endpoint, Example};

    fn basic(package: &str) -> Example {
        Example::new(package, "basic")
    }

    fn assert_table_output(package: &str, completion_msg: &str) {
        let stdout = basic(package).stdout();

        let all_expected = [
            completion_msg,
            "sql - SQL query execution time statistics.",
            // Sum of all bucket counts, including entries cut by the display
            // limit.
            "Total calls: 115",
            "INSERT INTO users (name, age) VALUES (?, ?)",
            // Short query (4 words) arrives via `summary`, not `db.statement`.
            "SELECT COUNT(*) FROM users",
            // Inline literals normalized into one bucket.
            "SELECT name FROM users WHERE age = ?",
            // Different-arity IN lists collapse to one bucket.
            "SELECT * FROM users WHERE id IN (?)",
        ];
        for expected in all_expected {
            assert!(
                stdout.contains(expected),
                "[{package}] Expected:\n{expected}\n\nGot:\n{stdout}",
            );
        }
    }

    fn assert_transaction_queries_captured(package: &str) {
        // 50 loop inserts + 1 transaction-internal insert = 51. A pool wrapper
        // would miss the transaction-internal query; the layer captures it.
        let stdout = basic(package).json().stdout();

        let all_expected = [
            "\"sql\"",
            "\"INSERT INTO users (name, age) VALUES (?, ?)\"",
            "\"count\":51",
            "\"total_calls\":115",
        ];
        for expected in all_expected {
            assert!(
                stdout.contains(expected),
                "[{package}] Expected:\n{expected}\n\nGot:\n{stdout}",
            );
        }
    }

    // With HOTPATH_SQL_RAW_LOGS=1, log entries keep the raw statement text -
    // inline literals included - while the bucket stays normalized.
    #[test]
    fn test_sql_raw_logs_opt_in() {
        let _running = basic("test-sqlx-08")
            .env("HOTPATH_METRICS_PORT", "6786")
            .env("HOTPATH_SQL_RAW_LOGS", "1")
            .env("TEST_SLEEP_SECONDS", "30")
            .spawn();

        let literal_query = "SELECT name FROM users WHERE age = ?";
        let json_text = poll_endpoint(
            "http://localhost:6786/sql",
            40,
            Duration::from_millis(750),
            |body| body.contains(literal_query),
        );

        let sql: JsonSqlList = serde_json::from_str(&json_text).expect("Failed to parse /sql");
        let entry = sql
            .data
            .iter()
            .find(|e| e.query == literal_query)
            .expect("inline-literal bucket not found in /sql");

        let logs_url = format!("http://localhost:6786/sql/{}/logs", entry.id.unwrap());
        let mut response = ureq::get(&logs_url)
            .call()
            .expect("Failed to call /sql/:id/logs endpoint");
        let body = response
            .body_mut()
            .read_to_string()
            .expect("Failed to read logs body");
        let logs: JsonSqlLogsList =
            serde_json::from_str(&body).expect("Failed to parse /sql/:id/logs");

        assert!(!logs.logs.is_empty(), "Expected log entries, got: {body}");
        // The example runs `... WHERE age = 21` through `... WHERE age = 40`.
        assert!(
            logs.logs
                .iter()
                .any(|l| l.query == "SELECT name FROM users WHERE age = 21"),
            "Expected raw inline literal in logs, got: {body}"
        );
    }

    #[test]
    fn test_table_output_sqlx_08() {
        assert_table_output("test-sqlx-08", "sqlx 0.8 tracing-layer example completed!");
    }

    #[test]
    fn test_table_output_sqlx_09() {
        assert_table_output("test-sqlx-09", "sqlx 0.9 tracing-layer example completed!");
    }

    #[test]
    fn test_transaction_queries_captured_sqlx_08() {
        assert_transaction_queries_captured("test-sqlx-08");
    }

    // Polls the live /sql endpoint, then fetches per-query execution logs from
    // /sql/{id}/logs and asserts they hold the normalized statement text.
    #[test]
    fn test_sql_logs_endpoint() {
        let _running = basic("test-sqlx-08")
            .env("HOTPATH_METRICS_PORT", "6785")
            .env("TEST_SLEEP_SECONDS", "30")
            .spawn();

        let insert_query = "INSERT INTO users (name, age) VALUES (?, ?)";
        let json_text = poll_endpoint(
            "http://localhost:6785/sql",
            40,
            Duration::from_millis(750),
            |body| body.contains(insert_query),
        );

        let sql: JsonSqlList = serde_json::from_str(&json_text).expect("Failed to parse /sql");
        let entry = sql
            .data
            .iter()
            .find(|e| e.query == insert_query)
            .expect("INSERT bucket not found in /sql");

        let logs_url = format!("http://localhost:6785/sql/{}/logs", entry.id.unwrap());
        let mut response = ureq::get(&logs_url)
            .call()
            .expect("Failed to call /sql/:id/logs endpoint");
        assert_eq!(response.status(), 200);

        let body = response
            .body_mut()
            .read_to_string()
            .expect("Failed to read logs body");
        let logs: JsonSqlLogsList =
            serde_json::from_str(&body).expect("Failed to parse /sql/:id/logs");

        assert!(!logs.logs.is_empty(), "Expected log entries, got: {body}");
        let log = &logs.logs[0];
        assert_eq!(log.query, insert_query);
        assert!(!log.duration.is_empty());
        assert!(!log.ago.is_empty());
        // 51 inserts total, capped at HOTPATH_LOGS_LIMIT (default 50).
        assert_eq!(logs.logs.len(), 50, "Got: {body}");
        assert_eq!(log.index, 51, "newest execution first, got: {body}");

        // Log entries hold the normalized text - the example executes
        // `SELECT name FROM users WHERE age = <literal>` with varying
        // inline literals, and none of them may leak into the logs.
        let literal_query = "SELECT name FROM users WHERE age = ?";
        let entry = sql
            .data
            .iter()
            .find(|e| e.query == literal_query)
            .expect("inline-literal bucket not found in /sql");
        let logs_url = format!("http://localhost:6785/sql/{}/logs", entry.id.unwrap());
        let mut response = ureq::get(&logs_url)
            .call()
            .expect("Failed to call /sql/:id/logs endpoint");
        let body = response
            .body_mut()
            .read_to_string()
            .expect("Failed to read logs body");
        let logs: JsonSqlLogsList =
            serde_json::from_str(&body).expect("Failed to parse /sql/:id/logs");
        assert!(!logs.logs.is_empty(), "Expected log entries, got: {body}");
        for log in &logs.logs {
            assert_eq!(log.query, literal_query, "literal leaked: {body}");
        }

        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let resp = agent
            .get("http://localhost:6785/sql/999999/logs")
            .call()
            .expect("Failed to call logs endpoint with unknown id");
        assert_eq!(resp.status().as_u16(), 404, "Expected 404 for unknown id");
    }

    #[test]
    fn test_transaction_queries_captured_sqlx_09() {
        assert_transaction_queries_captured("test-sqlx-09");
    }
}

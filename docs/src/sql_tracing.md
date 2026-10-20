# Rust SQL Performance Profiling for sqlx, SeaORM, Diesel and Toasty

<img loading="lazy" src="{{#asset-hash images/sql-report.png}}" alt="hotpath-rs terminal SQL report table showing normalized queries with source function attribution, call counts, average, P95, total, and percent-of-total execution time">

`hotpath` profiles SQL queries in Rust applications, helping you identify slow statements, repetitive query patterns, and unexpected database activity. Queries are grouped by their normalized SQL text, so parameterized executions of the same statement are reported together. For example, 1,000 executions of `SELECT * FROM users WHERE id = ?` appear as a single entry with call count, average latency, percentiles, and total execution time.

The same profiling backend powers sqlx (and SeaORM, which runs on top of it), Diesel, and the Toasty ORM, with more integrations coming soon. Instrumentation is inactive unless the `hotpath` feature is enabled.

## Normalizing SQL queries 

Queries are grouped by normalized text:

- single-quoted string literals become `?`
- positional placeholders (PostgreSQL `$1`, SQLite `?1`) become `?`
- numeric literals become `?`
- runs of `?` inside an `IN (...)` list collapse to `IN (?)`
- whitespace is squashed to single spaces

Only parameter-varied executions of the *same* statement merge - structurally different statements stay separate. So these two collapse into a single bucket (`SELECT * FROM users WHERE id IN (?)`):

```sql
SELECT * FROM users WHERE id IN (1, 2, 3)
SELECT * FROM users WHERE id IN (4, 5, 6, 7, 8)
```

Bound parameters never reach the report - only the statement shape does.

## Source function attribution

Each query is attributed to the innermost `#[hotpath::measure]`-instrumented function that was executing when the query ran - the `Source` column in the report and TUI. `hotpath` maintains a per-thread stack of instrumented function names: sync functions push their name on entry and pop on return, and async functions push and pop around every `poll`, so tasks interleaved on one runtime thread never report a stale caller.

Source is part of the grouping key: the same normalized statement executed from two different instrumented functions appears as two separate rows, so you can tell which code path is responsible for which share of the query load.

If no instrumented function is active when the query executes - the query runs from uninstrumented code, or inside a spawned task whose functions aren't instrumented - the `Source` column shows `-`. To get attribution, annotate the functions that issue queries (or their callers) with `#[hotpath::measure]`.

In axum applications wrapped with [`hotpath::axum!`](axum_tracing.md#route-scoping-for-sql-and-http) queries are additionally grouped by the route that issued them, shown in a `Route` column.

## Profiling sqlx queries with a tracing layer

Add `hotpath` with the `sqlx` feature to your `Cargo.toml`:

```toml
[dependencies]
hotpath = { version = "{{HOTPATH_VERSION}}", features = ["sqlx"] }
```

`hotpath` uses `tracing_subscriber::Layer` to capture `sqlx` query events with their timing info. Configure it like this:

```rust
use tracing_subscriber::prelude::*;

tracing_subscriber::registry()
    .with(hotpath::sqlx_tracing_layer())
    .init();
```

That's it - every query executed through any `sqlx` pool or connection is now profiled.

### EnvFilter caveat

A *global* `EnvFilter` (`registry().with(env_filter)`) runs before the hotpath layer's own filter and can suppress the `sqlx::query` events for the whole stack, emptying the SQL report. Attach any `EnvFilter` **per-layer** instead, or make sure you don't globally filter out the `sqlx::query` target.

### SeaORM

[SeaORM](https://www.sea-ql.org/SeaORM/) executes every statement through a `sqlx` pool, so the same `sqlx` feature and `hotpath::sqlx_tracing_layer()` profile it with no extra setup. Queries generated from entities (`insert`, `find_by_id`, `filter`, `count`), raw `Statement`s, and transaction-internal queries are all captured.

SeaORM forwards its `sqlx_logging` connect option to sqlx, so keep it enabled (the default). Connecting with `ConnectOptions::sqlx_logging(false)` turns off the `sqlx::query` events and the SQL report stays empty.

## Profiling Diesel queries with Instrumentation

Add `hotpath` with the `diesel` feature to your `Cargo.toml`:

```toml
[dependencies]
hotpath = { version = "{{HOTPATH_VERSION}}", features = ["diesel"] }
```

Diesel emits nothing through `tracing`, so instead of a layer it exposes a custom callback trait. Install `hotpath`'s instrumentation before opening connections:

```rust
hotpath::instrument_diesel_sql();

// open connections AFTER this call so they pick up the instrumentation
let mut conn = SqliteConnection::establish(":memory:")?;
// or any other Diesel backend:
let mut conn = PgConnection::establish(&database_url)?;
```

`instrument_diesel_sql()` registers the instrumentation as the default for every newly-established connection. Connections established *before* the call are not instrumented.

- **Backend coverage is automatic** - the trait lives in Diesel core, so Postgres, MySQL, and SQLite are all covered. Enable the matching Diesel backend feature in your own crate.
- **Transaction control statements** (`BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`) are filtered out - the report stays queries-only. Queries *inside* a transaction are captured.
- **Synchronous connections only.** `instrument_diesel_sql()` registers Diesel's global default instrumentation, which covers `diesel::Connection` types. `diesel_async` support is coming soon.

## Profiling Toasty ORM queries with a tracing layer

Add `hotpath` with the `toasty` feature to your `Cargo.toml`:

```toml
[dependencies]
hotpath = { version = "{{HOTPATH_VERSION}}", features = ["toasty"] }
```

Every [Toasty](https://github.com/tokio-rs/toasty) driver emits one `tracing` event per physical database operation, and `hotpath` captures them the same way it does for sqlx - with a `tracing_subscriber::Layer`:

```rust
use tracing_subscriber::prelude::*;

tracing_subscriber::registry()
    .with(hotpath::toasty_tracing_layer())
    .init();
```

That's it - every query executed through a Toasty `Db`, `Connection`, or `Transaction` is now profiled, whether it was generated from a model (`create!`, `filter_by_...`) or written as raw SQL (`toasty::sql::query`).

- **SQL backend coverage is automatic** - the event is emitted by toasty-core, so SQLite, PostgreSQL, MySQL, and Turso are all covered. Key-value drivers (DynamoDB) execute no SQL and are skipped.
- **Timing comes from Toasty itself** - the layer reads the `duration_ms` field Toasty measures at the driver level, so transaction-internal queries are captured too.
- The same `EnvFilter` caveat as sqlx applies: don't globally filter out the `toasty::query` target.

## Limiting and capping query output

The number of queries shown is unlimited by default (`0`). Cap it with:

- Builder: `.sql_limit(n)`
- Env var: `HOTPATH_SQL_LIMIT`

## Live SQL metrics 

Live SQL queries metrics display in the `I/O -> SQL` TUI tab:

<img loading="lazy" src="{{#asset-hash images/sql-query-execution-time.png}}" alt="hotpath-rs SQL report showing per-query execution time, call counts, and percentiles for normalized sqlx and Diesel queries">

### Raw statement logs (opt-in)

Set `HOTPATH_SQL_RAW_LOGS=1` to store the raw statement text in execution logs instead of the normalized form. Queries still merge into normalized buckets, but each log entry then shows the exact statement as sent to the database - so parameter-varied executions of one bucket stay distinguishable in the logs panel and inspect popup.

**Privacy caveat:** raw text exposes inline literals (values rendered into the SQL string, e.g. via `format!`) through the TUI and the unauthenticated local metrics server. Bound parameters (`.bind(...)`) are never visible either way - the driver only ever reports the placeholder form.

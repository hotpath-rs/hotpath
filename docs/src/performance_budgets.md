# Performance budgets: absolute limits for Rust functions, queries and routes

A performance budget is a hard limit on one function, query, route or other profiled entry: `"this function never allocates more than 1 KB per call"`, `"this query runs at most once per request"`. The [regression policy](regression_policy.md) compares a pull request with its baseline, so slow drift over many pull requests passes it. A budget is checked against each report alone, without a baseline, so drift cannot slip past it. It also applies to the first pull request and to pushes to the default branch.

Budgets live in the same file as the [regression policy](regression_policy.md) rules, `hotpath/policy.toml`. A broken budget fails the verdict like a regression.

## Rules

One `[[<resource>.budgets]]` table per rule, under the resource it limits.

### Functions

Allocations, timing and call count of [instrumented functions](functions.md).

```toml
[[functions.budgets]]
match = "my_crate::parser::parse"
alloc = { avg = "1 KB", total = "10 MB" }
timing = { avg = "250 us", p95 = "1 ms" }
calls = { min = 1000, max = 1000 }
message = "parse must stay allocation-light"
```

`alloc` takes bytes and `timing` durations, for `avg`, `total` and percentiles. `match` is the function name.

### SQL queries

Timing and execution count of [SQL queries](sql_tracing.md).

```toml
[[sql.budgets]]
match = "SELECT * FROM users WHERE id = ?*"
timing = { avg = "2 ms", total = "50 ms" }
count = { max = 100 }
```

`match` runs against `query | route | source`, so a pattern on the query alone, ending with `*`, matches it wherever it ran.

### HTTP requests

Timing, errors and request count of [outbound HTTP requests](http_tracing.md).

```toml
[[http.budgets]]
match = "*api.stripe.com*"
timing = { avg = "300 ms", errors = 0 }
count = { max = 20 }
```

`match` runs against `endpoint | route | source`.

### Server routes

Allocations, response time and downstream calls of each [served route](axum_tracing.md).

```toml
[[server.budgets]]
match = "GET /users/{id}"
alloc = { bytes_per_request = 65536, allocs_per_request = 200 }
timing = { p95 = "50 ms", status_5xx = 0, sql_per_request = 1 }
```

`timing` takes `avg`, `total` and percentiles as durations, `status_4xx` and `status_5xx` as counts, `sql_per_request` and `http_per_request` as numbers. `alloc` takes `avg`, `total` and percentiles as bytes, `bytes_per_request` and `allocs_per_request` as numbers. `match` is the route as `METHOD template`.

### Mutexes

Wait and hold time of [mutexes](locks.md).

```toml
[[mutexes.budgets]]
match = "app_state"
timing = { wait_avg = "100 us", acquire_avg = "1 ms" }
```

`match` is the lock's label.

### RwLocks

Wait and hold time of [read-write locks](locks.md), for reads and writes.

```toml
[[rw_locks.budgets]]
match = "config_cache"
timing = { read_wait_avg = "50 us", write_acquire_avg = "1 ms" }
```

`match` is the lock's label. `count` is reads and writes together.

### Channels

Delay, backlog and throughput of [channels](data_flow.md).

```toml
[[channels.budgets]]
match = "events"
flow = { delay_avg = "1 ms", max_queue_size = 1000, received_per_sec = 5000 }
sent_count = { min = 10000 }
```

`sent_per_sec` and `received_per_sec` are **minimums**: the channel must move at least that many messages per second. The rest are maximums. The count is `sent_count`.

### I/O

Latency, throughput and errors of [I/O streams](io_tracing.md).

```toml
[[io.budgets]]
match = "upload_socket*"
flow = { avg = "5 ms", errors = 0, bytes_per_sec = "10 MB" }
```

`bytes_per_sec` is a **minimum**, the rest are maximums. `match` runs against `label | operation | -`, so a pattern on the label alone matches every operation on it.

## Options

### Rule keys

Every rule takes:

| Key | Meaning |
|---|---|
| `match` | The entry the rule applies to. `*` matches any run of characters, and the pattern must match the whole name. Every entry it matches is checked on its own. |
| `calls` / `count` / `sent_count` | `{ min, max }` on how many times the entry ran: `calls` for functions, `sent_count` for channels, `count` for the rest. |
| `alloc`, `timing`, `flow` | Limits on the resource's metrics, as in [Rules](#rules). |
| `message` | Optional text shown next to a broken rule in the comment, up to 256 bytes. |
| `fail_check` | Whether a broken rule fails the [pull request check](regression_policy.md#pull-request-check). `true` by default; `false` reports the rule without failing the check. |

**Every rule checks that its entry ran.** Without a `min`, the count must be at least 1, so a `match` with a typo or a renamed function breaks the budget instead of passing silently. Write `calls = { min = 0 }` for an entry that may not run.

`ignore`, `judged`, `min_percent_change`, `min_calls` and `min_percent_total` do not apply to budgets: a rule names its entry explicitly. A resource takes up to 64 rules.

### Units

Values carry their unit:

| Kind | Written as |
|---|---|
| durations | `"250 ns"`, `"250 us"`, `"5 ms"`, `"1.5 s"` |
| bytes | `"512 B"`, `"1 KB"`, `"10 MB"` (1 KB = 1024 B) |
| bytes per second | `"10 MB"`, `"500 B"` |
| counts | a whole number: `0`, `1000` |
| per-request numbers | a number: `1`, `2.5` |

A percentile name with a dot is quoted: `timing = { "p99.9" = "5 ms" }`.

## Broken budgets

A broken budget is listed under "Budgets" in the pull request comment, with the value, the limit and the rule's `message`, and the heading counts it. Under `pr_check = "fail"` a rule with `fail_check` (the default) says it fails the check:

```
⛔ my_crate::parser::parse: alloc.avg 1.4 KB, over budget 1.0 KB: parse must stay allocation-light · fails the check
⛔ my_crate::flush: not called · fails the check
```

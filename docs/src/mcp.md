# AI-powered Rust profiling: query performance metrics with LLMs via MCP

The Model Context Protocol (MCP) integration allows LLM agents like Claude Code to query your Rust application's performance metrics in real-time. Instead of manually reading profiling reports, you can ask questions like:

- `"Which functions dominate total execution time right now?"`
- `"Where is most of my memory being allocated?"`
- `"Which mpsc channel has the highest throughput and queue pressure?"`

and get instant answers.

## Setup

Run your program with the MCP enabled alongside profiling:

```bash
cargo run --features='hotpath-alloc,hotpath-mcp'
```

## Available tools

### Summary tools

These tools return aggregated performance summaries:

| Tool | Description |
|------|-------------|
| `functions_timing` | Function timing statistics (calls, avg, percentiles, total time) |
| `functions_alloc` | Function memory allocation statistics (requires `hotpath-alloc` feature) |
| `functions_cpu` | CPU sampling attribution per function (requires `hotpath-cpu` feature) |
| `functions_cpu_snapshot` | Trigger an on-demand CPU sampling snapshot (requires `hotpath-cpu` feature) |
| `channels` | Channel metrics (sends, receives, throughput, queue size) |
| `streams` | Stream metrics (items yielded, state) |
| `futures` | Future execution statistics (calls, polls, poll time) |
| `rw_locks` | RwLock read/write wait & acquire-time statistics |
| `mutexes` | Mutex wait & acquire-time statistics |
| `io` | Byte-level I/O statistics per wrapped reader/writer (requires `hotpath::io!(expr)`) |
| `sql` | SQL query execution-time statistics per normalized query (requires `sqlx_tracing_layer()`, `toasty_tracing_layer()` or `instrument_diesel_sql()`) |
| `http` | Outgoing HTTP request statistics per normalized endpoint (requires `hotpath::http!(client)` on a reqwest client or ureq config builder) |
| `server` | Served request response-time statistics per axum route (requires `hotpath::axum!(router)`) |
| `threads` | Per-thread CPU usage and allocations, plus process RSS |
| `gauges` | All `gauge!` entries with their current values |
| `dbg_entries` | All `dbg!` debug entries |
| `val_entries` | All `val!` value tracking entries |
| `tokio_runtime` | Tokio runtime metrics snapshot (requires `tokio` feature) |
| `profiler_status` | Profiler uptime and process id |

### Detailed log tools

These tools take an id returned by the matching summary tool and retrieve recent individual events:

| Tool | Parameters | Description |
|------|------------|-------------|
| `function_timing_logs` | `function_id` | Individual timing logs for a function (id from `functions_timing`) |
| `function_alloc_logs` | `function_id` | Individual allocation logs for a function (id from `functions_alloc`) |
| `channel_logs` | `channel_id` | Channel send/receive logs |
| `stream_logs` | `stream_id` | Stream yield logs |
| `future_logs` | `future_id` | Future call and poll logs |
| `sql_logs` | `sql_id` | Individual execution logs for a SQL query |
| `http_logs` | `http_id` | Individual request logs for an outgoing HTTP endpoint |
| `server_logs` | `server_id` | Individual request logs for a served axum route |
| `gauge_logs` | `gauge_id` | Gauge value history |
| `dbg_logs` | `debug_id` | Detailed logs for a `dbg!` entry (id from `dbg_entries`) |
| `val_logs` | `debug_id` | Detailed logs for a `val!` entry (id from `val_entries`) |

## Configuration

| Environment Variable | Default | Description |
|---------------------|---------|-------------|
| `HOTPATH_MCP_PORT` | `6771` | Port the MCP server listens on |
| `HOTPATH_MCP_AUTH_TOKEN` | - | Optional authentication token for secure access |

Example with custom configuration:

```bash
HOTPATH_MCP_PORT=8080 HOTPATH_MCP_AUTH_TOKEN=secret123 \
cargo run --features='hotpath-mcp'
```

## Claude Code setup

### Basic setup (no authentication)

```bash
claude mcp add --transport http hotpath http://localhost:6771/mcp
```

```json
"mcpServers": {
    "hotpath": {
        "type": "http",
        "url": "http://localhost:6771/mcp"
    }
}
```

### With authentication

If you've set `HOTPATH_MCP_AUTH_TOKEN`:

```bash
claude mcp add --transport http hotpath http://localhost:6771/mcp --header "Authorization: your-secret-token"
```

```json
"mcpServers": {
    "hotpath": {
        "type": "http",
        "url": "http://localhost:6771/mcp",
        "headers": {
            "Authorization": "your-secret-token"
        }
    }
}
```

After that, you can ask the model to answer performance questions directly using live profiling data.

## Example queries

Once connected, you can ask LLMs questions about your running application:

- `"Which functions dominate total execution time right now?"`
- `"Where is most of my memory being allocated?"`
- `"Which mpsc channel has the highest throughput and queue pressure?"`
- `"Are any functions showing high p95 or p99 latency spikes?"`
- `"Show the last 10 execution timing events for process_request function"`
- `"Compare latency and allocation behavior of sync_handler vs async_handler methods"`
- `"Which threads are currently consuming the most CPU and memory?"`
- `"Are any threads steadily allocating memory without deallocating it?"`

Agent will use the appropriate MCP tools to query your application's profiling data and provide insights.

<img loading="lazy" src="{{#asset-hash images/mcp-analysis.png}}" alt="hotpath-rs MCP integration showing LLM agent querying Rust profiling data">

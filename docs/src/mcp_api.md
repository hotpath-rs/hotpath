# hotpath-cloud MCP Server and REST API

The hotpath-cloud MCP server connects AI assistants such as Claude, ChatGPT, Codex and Cursor to the Rust profiling reports your CI uploads. With it, an assistant can:

- analyze stored profiling and benchmark reports,
- investigate a performance regression in a pull request,
- compare a CI benchmark run with its baseline,
- find the functions, queries and routes that broke a [performance budget](performance_budgets.md),
- follow a benchmark's history across commits and runners,
- read and validate [regression policies](regression_policy.md).

The same data is available three ways: the remote MCP server for AI assistants, a JSON REST API for scripts and automation, and the [`hotpath cloud`](agents_cli.md) CLI. All three are read-only: they cannot upload reports, delete data or change repository settings. Reports get there through [CI integration](ci_integration.md).

This server is hosted and serves stored CI reports. It is a different server from the [live profiling MCP server](mcp.md) of the hotpath profiler, which runs inside your Rust application and answers about the running process.

## Server URL and authentication

```
https://hotpath.rs/api/v1/mcp
```

The server speaks Streamable HTTP. A client can connect and list the tools without signing in; every tool call needs a credential, also for repositories whose reports are public. There are two kinds:

- **OAuth sign-in.** When a client first needs your data, hotpath.rs asks you to log in with GitHub and shows a consent page that names the application. After you approve it, the application can use the [tools](#available-mcp-tools) on behalf of your account, with the single read-only scope `hotpath:read`. Clients identify themselves with an OAuth Client ID Metadata Document; dynamic client registration is not supported.
- **Personal API token.** For clients without that kind of OAuth support, create a token at <a href="https://hotpath.rs/app/tokens" target="_blank" rel="noopener noreferrer">hotpath.rs/app/tokens</a> and send it as `Authorization: Bearer <token>`.

### Which repositories you can read

Access follows GitHub and the repository's reports visibility on the <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">dashboard</a>. A repository with the hotpath GitHub App installed is readable when:

- its reports are public (the default for a public repository): any signed-in user can read them by naming the repository, also when it is not one of theirs,
- it is private on GitHub and your GitHub user can read it,
- it is public on GitHub with private reports, and your GitHub user has push permission to it.

`list_repos` (and `GET /repos`) is not a directory of every project using hotpath-cloud. It lists only the repositories in the GitHub App installations your GitHub user can see, the same list as <a href="https://hotpath.rs/app" target="_blank" rel="noopener noreferrer">hotpath.rs/app</a>. To read the public reports of another project, pass its `owner/name` to the other tools directly. A repository you cannot read answers `not_found`, the same as one that does not exist.

## Connect your AI assistant

Client setup screens change often; each section links to the client's own guide.

### Claude and Claude Desktop

1. Open **Customize > Connectors**, select **+ Add**, then **Add custom connector**.
2. Enter a name, such as `hotpath-cloud`, and the server URL, then select **Continue**.
3. Keep the detected OAuth sign-in with Claude's published identity, and select **Add**.
4. Approve the connection on the hotpath.rs consent page.

The connector works in claude.ai, Claude Desktop and Cowork. On Team and Enterprise plans an Owner adds it first in **Organization settings > Connectors**, and members then select **Connect** under **Customize > Connectors**. See Anthropic's guide to <a href="https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp" target="_blank" rel="noopener noreferrer">custom connectors</a>.

### Claude Code

```bash
claude mcp add --transport http --scope user hotpath-cloud https://hotpath.rs/api/v1/mcp
```

Then run `/mcp` in a Claude Code session, select `hotpath-cloud` and log in in the browser. `claude mcp list` shows whether the server is connected or still needs authentication.

`--scope` decides where the server loads:

| Scope | Loads in | Stored in |
|---|---|---|
| `user` | all your projects, private to you | `~/.claude.json` |
| `local` (default) | the current project only, private to you | `~/.claude.json` |
| `project` | the current project, shared with everyone who uses it | `.mcp.json` in the project root, committed to Git |

OAuth is the recommended sign-in. To use a personal API token instead, keep the token in an environment variable and let Claude Code expand it when it connects, so the configuration holds only the variable name:

```bash
claude mcp add-json --scope user hotpath-cloud '{"type":"http","url":"https://hotpath.rs/api/v1/mcp","headers":{"Authorization":"Bearer ${HOTPATH_API_TOKEN}"}}'
```

The single quotes keep your shell from expanding `${HOTPATH_API_TOKEN}`, so the token itself is never written to `~/.claude.json`. Set `HOTPATH_API_TOKEN` in the environment you start Claude Code from. The same entry works in a project's `.mcp.json`, where every teammate supplies their own token and none is committed. See the <a href="https://code.claude.com/docs/en/mcp" target="_blank" rel="noopener noreferrer">Claude Code MCP docs</a>.

### ChatGPT

Custom MCP servers are available in ChatGPT Business, Enterprise and Edu workspaces. Workspace admins control who can add them and where they are available.

1. Open **Plugins** and select **Add custom MCP server**.
2. Enter the server URL and choose OAuth authentication.
3. Approve the connection on the hotpath.rs consent page.

See OpenAI's guide to <a href="https://help.openai.com/en/articles/12584461-connecting-custom-mcp-servers-in-chatgpt" target="_blank" rel="noopener noreferrer">connecting custom MCP servers</a> for the permissions involved.

### Codex

```bash
codex mcp add hotpath-cloud --url https://hotpath.rs/api/v1/mcp
codex mcp login hotpath-cloud
```

`codex mcp login` opens the browser for the hotpath.rs consent page; on a machine without a browser, add `--no-browser`. To use a personal API token instead, add the server to `~/.codex/config.toml`:

```toml
[mcp_servers.hotpath-cloud]
url = "https://hotpath.rs/api/v1/mcp"
bearer_token_env_var = "HOTPATH_API_TOKEN"
```

The Codex CLI, IDE extension and ChatGPT desktop app share this configuration. See the <a href="https://learn.chatgpt.com/docs/extend/mcp" target="_blank" rel="noopener noreferrer">Codex MCP docs</a>.

### Cursor and other clients

Cursor's OAuth uses dynamic client registration or a pre-registered client, which hotpath.rs does not offer, so connect it with a personal API token. In `~/.cursor/mcp.json` (all projects) or `.cursor/mcp.json` (one project):

```json
{
  "mcpServers": {
    "hotpath-cloud": {
      "url": "https://hotpath.rs/api/v1/mcp",
      "headers": { "Authorization": "Bearer ${env:HOTPATH_API_TOKEN}" }
    }
  }
}
```

Cursor fills `${env:HOTPATH_API_TOKEN}` from your environment. See the <a href="https://cursor.com/docs/mcp" target="_blank" rel="noopener noreferrer">Cursor MCP docs</a>. Any other MCP client works the same way: OAuth when it supports Client ID Metadata Documents, otherwise the bearer token header.

## Example prompts

- "What caused the performance regression in PR #742 of hotpath-rs/hotpath?"
- "Compare the last five main branch runs of the drain benchmark in hotpath-rs/hotpath. Are allocations increasing?"
- "Which budgets did the latest CI report of my-org/my-repo break, and by how much?"
- "Compare the drain benchmark of hotpath-rs/hotpath on Linux-ARM64 and Linux-X64 runners, using the `runner` report metadata."
- "Show recent reports for hotpath-rs/hotpath and identify any regressions."
- "Write a policy that judges allocations at 10% and check that it is valid."

For questions about a particular repository's benchmarks or reports, include its full `owner/name`, especially when querying another project's public reports.

## Available MCP tools

Every tool is read-only.

| Tool | What it does |
|---|---|
| `whoami` | Shows which GitHub user and token the connection acts as |
| `list_repos` | Lists the repositories in your GitHub App installations, each with its benchmarks. `with_benchmarks: true` lists only profiled ones, `query` filters by name |
| `list_benchmarks` | Lists one repository's benchmarks |
| `list_reports` | Lists a repository's reports, newest first, filtered by benchmark, event, pull request, commit, branch, time window, verdict or report metadata, with cursor pagination |
| `get_report` | Fetches one report by pull request, commit or id, with its verdict, policy file and dashboard URL, and optionally the uploaded hotpath JSON report |
| `get_diff` | Compares a report with its baseline under the policy it carried, exactly as the pull request comment did: what regressed, what improved, which [budgets](performance_budgets.md) broke |
| `get_default_policy` | Returns the default [regression policy](regression_policy.md) file |
| `validate_policy` | Checks that a policy TOML document parses, with the same parser that judges uploaded reports |

The answers have the same shape as the CLI's JSON output, and the arguments mean the same as its flags; [AI Agents CLI](agents_cli.md#cli-commands) explains both, including how to read a diff.

`get_report`, `list_reports` and `get_diff` accept `select`, which names entities per resource with exact `keys`, such as `{"functions":{"keys":["app::hot"]},"io":{"keys":["sock"]}}`. It filters the data the answer already carries and adds no field such as `metrics`:

- A report keeps its payload sections, each with only the matching `data` entries. `get_report` includes the payload when `select` is given.
- `list_reports` without `select` lists summaries only; with it, each entry is the report with its filtered payload, as `get_report` returns it.
- A diff keeps only the matching comparison rows. `select` combines with `rows`, which defaults to `all` when `select` is given.
- I/O is selected by the value's label and keeps every operation of it.

[AI Agents CLI](agents_cli.md#report) has examples.

## REST API

The base URL is `https://hotpath.rs/api/v1`. Every request needs a personal API token from <a href="https://hotpath.rs/app/tokens" target="_blank" rel="noopener noreferrer">hotpath.rs/app/tokens</a> in the `Authorization` header; OAuth tokens issued to MCP clients do not work here. Repository access is the same as for [the MCP server](#which-repositories-you-can-read).

```bash
curl -H "Authorization: Bearer $HOTPATH_API_TOKEN" https://hotpath.rs/api/v1/auth
```

```json
{"login":"pawurb","token":{"name":"laptop","expires_at":"2026-12-25T11:27:12.311073Z"}}
```

The five newest reports of `hotpath-rs/hotpath`, across all its benchmarks:

```bash
curl -H "Authorization: Bearer $HOTPATH_API_TOKEN" \
  "https://hotpath.rs/api/v1/repos/hotpath-rs/hotpath/reports?limit=5"
```

Trimmed answer:

```json
{
  "repository": "hotpath-rs/hotpath",
  "reports": [
    {
      "id": "01a1b2de-8cf9-7a91-a291-fe44b90015da",
      "benchmark": "random",
      "event": "pull_request",
      "commit_sha": "71b98a75585f1a7cc78ec809ca624b80baef1c0b",
      "pr_number": 742,
      "baseline_id": "01a1b2d3-df76-7591-a393-0fca372086a7",
      "regressed": true,
      "failed": false,
      "policy_path": "hotpath/random-policy.toml",
      "created_at": "2026-11-06T21:58:11.954835Z",
      "dashboard_url": "https://hotpath.rs/app/repos/hotpath-rs/hotpath/benchmarks/random/reports/01a1b2de-8cf9-7a91-a291-fe44b90015da"
    }
  ],
  "next_cursor": "AQGhstPfdnWRo5MPyjcghqc"
}
```

| Endpoint | Answers |
|---|---|
| `GET /auth` | the user and token of the request |
| `GET /repos` | the repositories in your installations, with their benchmarks; `query=TEXT`, `with_benchmarks=true` |
| `GET /repos/{owner}/{name}/benchmarks` | one repository's benchmarks |
| `GET /repos/{owner}/{name}/reports` | the repository's reports, newest first; `benchmark`, `event`, `pr`, `commit`, `branch`, `since`, `until`, `verdict`, `metadata`, `select`, `limit`, `cursor` |
| `GET /repos/{owner}/{name}/benchmarks/{benchmark}/reports/latest` | the newest report of `pr=N` or `commit=SHA`, optionally narrowed by `event`; `payload=false` leaves out the uploaded report, `select` filters it |
| `GET /repos/{owner}/{name}/benchmarks/{benchmark}/reports/{id}` | one report by id; `payload=false` and `select` as above |
| `GET /repos/{owner}/{name}/benchmarks/{benchmark}/diff` | the diff of the report picked by `pr=N`, `commit=SHA` or `head=ID`, optionally narrowed by `event`; `rows=findings\|advisory\|all`, `select` |
| `GET /policy/default` | the default policy file, as `source` |
| `POST /policy/validate` | checks the policy sent as the JSON body `{"source": "<TOML>"}` |

The parameters mean the same as the CLI flags of the same name in [AI Agents CLI](agents_cli.md#cli-commands). `limit` (1 to 50, default 20) and `cursor` page through reports: pass the answer's `next_cursor` as `cursor` with the same filters. `metadata` is a JSON object of string values, such as `metadata={"runner":"Linux-ARM64"}` (URL-encoded). `select` is the JSON object described [above](#available-mcp-tools), URL-encoded; it cannot be combined with `payload=false`, and with it a listing page holds 1 to 20 reports (default 10).

Every error answers a non-2xx status with a JSON body that names the problem and a machine-readable `code`:

```json
{"error":"No report for PR #999999 in benchmark drain.","code":"not_found"}
```

## Permissions and token management

- **Connected applications.** Every application you approved over OAuth is listed on <a href="https://hotpath.rs/app/tokens" target="_blank" rel="noopener noreferrer">hotpath.rs/app/tokens</a>. Revoking one deletes its tokens at once; it can return only after a new consent.
- **Personal API tokens.** Created, listed and revoked on the same page. A token reads exactly what your account reads.
- **Nothing to grant beyond reading.** There is no write scope. Uploads come only from CI, authenticated by the workflow's GitHub OIDC token, as [CI integration](ci_integration.md) describes.

See the [privacy policy and terms of service](terms.md), including what a connected application can access. Questions and support: support@hotpath.rs.

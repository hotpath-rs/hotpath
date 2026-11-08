//! `hotpath cloud ...`: the hotpath.rs API client. One file per command under
//! `cloud/`, shared HTTP / auth / output plumbing in `cloud/api.rs`.

mod api;
mod auth;
mod benchmarks;
mod diff;
mod init;
mod policy;
mod repo;
mod report;
mod reports;
mod repos;
mod select;
mod selector;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::cmd::cloud::api::{CliError, Output};
use crate::cmd::cloud::diff::DiffArgs;
use crate::cmd::cloud::init::InitArgs;
use crate::cmd::cloud::policy::ValidatePolicyArgs;
use crate::cmd::cloud::report::ReportArgs;
use crate::cmd::cloud::reports::ReportsArgs;

#[derive(Parser, Debug)]
pub(crate) struct CloudArgs {
    #[command(subcommand)]
    pub(crate) cmd: CloudCommand,

    #[arg(long, global = true, help = "Indent the JSON output")]
    pub(crate) pretty: bool,

    #[arg(
        long,
        global = true,
        value_name = "FILE",
        help = "Write the JSON output to FILE instead of stdout"
    )]
    pub(crate) output: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub(crate) enum CloudCommand {
    #[command(
        about = "Show which user and token this shell acts as (GET /api/v1/auth)",
        long_about = "Show which user and token this shell acts as (GET /api/v1/auth).

Reads the token from HOTPATH_API_TOKEN (create one at https://hotpath.rs/app/tokens)
and the base URL from HOTPATH_API_URL (default https://hotpath.rs). Exits 1 with the
server's error JSON on stderr when the token does not work."
    )]
    Auth,

    #[command(
        about = "List the repositories the token reaches, with their benchmarks (GET /api/v1/repos)",
        long_about = "List the repositories the token reaches, with their benchmarks (GET /api/v1/repos).

Every active repository the token's user can see on GitHub with the hotpath App
installed, ordered by full name, each with its benchmarks (name, stored reports, time
of the newest report). Same token and base URL environment as `auth`."
    )]
    Repos,

    #[command(
        about = "List one repository's benchmarks (GET /api/v1/repos/{owner}/{name}/benchmarks)",
        long_about = "List one repository's benchmarks (GET /api/v1/repos/{owner}/{name}/benchmarks).

The repository is --repo owner/name, as `repos` lists it. A repository that does not
exist, that the token's user cannot see or that has no App installed all answer 404.
Same token and base URL environment as `auth`."
    )]
    Benchmarks {
        #[arg(long, value_name = "OWNER/NAME", help = "The repository")]
        repo: String,
    },

    #[command(
        about = "Fetch one stored report by pull request, commit or id",
        long_about = "Fetch one stored report by pull request, commit or id
(GET /api/v1/repos/{owner}/{name}/benchmarks/{benchmark}/reports/latest?pr=N|commit=SHA,
or .../reports/{id}).

--pr and --commit answer the newest matching report, newest by upload. --commit takes
a full 40-character sha and matches the measured commit or the pull request's head
commit: a pull request job usually measures GitHub's merge commit, which nobody has
locally, so `--commit $(git rev-parse HEAD)` on a PR branch still finds the PR's
report. --event push|pull_request narrows --pr / --commit to one event (the same
commit is often measured by a push to main and as a PR head). --no-payload asks for
the summary only; the payload is the uploaded hotpath JSON report without
`meta.policy`.

--select JSON filters the payload to the named entities, e.g.
'{\"functions\":{\"keys\":[\"app::hot\",\"app::process\"]},\"io\":{\"keys\":[\"sock\"]}}'.
Keys are exact (as the diff's rows name them; io takes the value's label, every op of
it; threads the name, every thread of it). Each kept section's `data` holds only the
matching entries, unchanged, with `included_count` set to their number; `total_count`
and the totals still describe the whole run. Sections of other resources, `streams`,
`futures`, `debug` and named sections left empty are dropped, so a key that matches
nothing keeps nothing. Not with --no-payload. A payload the server cannot read
answers 400 rather than an unfiltered one.

`policy_path` names the policy file the report was judged under, relative to the
repository root, and `policy_url` is that file on GitHub, pinned to the measured
commit. Every stored report carries its policy, so both are always set. The document is the file at that path in the measured
commit; the API never returns it, with or without --no-payload.

A report of another benchmark, an unknown id, no match yet and a repository the
token's user cannot see all answer 404 `not_found`: poll with --commit until it
exits 0 to wait for CI's upload. Same token and base URL environment as `auth`."
    )]
    Report(ReportArgs),

    #[command(
        about = "List a repository's stored reports, newest first, one page per run (GET /api/v1/repos/{owner}/{name}/reports)",
        long_about = "List a repository's stored reports, newest first, one page per run
(GET /api/v1/repos/{owner}/{name}/reports).

Each entry is one upload, as the summary `report --no-payload` prints (the payload
only with --select), so a pull request measured three times lists three reports. The
body is `repository`, `reports` and `next_cursor`.

Every filter is optional and they combine with AND:
  --benchmark NAME          one benchmark; without it, every benchmark of the repository
  --event EVENT             push or pull_request
  --pr N                    reports of this pull request
  --commit SHA              a full 40-character sha, matching the measured commit or the
                            pull request's head commit, so `--commit $(git rev-parse HEAD)`
                            on a PR branch finds the PR's reports; with --pr, the reports
                            of that PR at that commit
  --branch NAME             the bare branch name measured (refs/heads/NAME), not a PR's
                            head or base branch; a value starting with refs/ is refused
  --since DATE              uploaded at or after DATE
  --until DATE              uploaded strictly before DATE
  --verdict VERDICT         passed: judged without regressions; regressed: judged with
                            regressions, failed reports included; failed: judged and
                            failed; not_judged: no stored judgement
DATE is YYYY-MM-DD (UTC midnight) or an RFC 3339 time (2026-11-01T10:30:00+01:00);
--since must be before --until.

--select JSON lists each report exactly as `report --select` prints it, with its
payload filtered to the named entities; a page then holds 1 to 20 reports (default
10), each payload being read whole.

Pagination: --limit N reports per page, 1 to 50 (default 20). When more reports match,
`next_cursor` is a string: pass it as --cursor with the same filters to get the next
page; it is null on the last page. The cursor is opaque, passed back unchanged, and a
cursor the server did not issue answers 400. No run fetches more than one page.

Examples:
  hotpath cloud reports --repo OWNER/NAME --pretty
  hotpath cloud reports --repo OWNER/NAME --benchmark NAME --event push --branch main --limit 50
  hotpath cloud reports --repo OWNER/NAME --verdict regressed --since 2026-11-01
  hotpath cloud reports --repo OWNER/NAME --commit $(git rev-parse HEAD)
  hotpath cloud reports --repo OWNER/NAME --benchmark NAME --select '{\"functions\":{\"keys\":[\"app::hot\"]}}'

Exit codes:
  0  a page was listed, empty or with regressed or failed reports: a verdict is
     never an exit code here (use `diff` for that)
  1  any error (unknown repository or benchmark, bad cursor, bad argument value,
     auth, network, unparseable body)
  2  usage error
Same token and base URL environment as `auth`."
    )]
    Reports(Box<ReportsArgs>),

    #[command(
        about = "Compare a stored report with its baseline under the policy it carried",
        long_about = "Compare a stored report with its baseline under the policy it carried
(GET /api/v1/repos/{owner}/{name}/benchmarks/{benchmark}/diff?pr=N|commit=SHA|head=ID).

The head report is selected exactly as `report` selects one: --pr and --commit
answer the newest matching report, newest by upload; --commit takes a full
40-character sha and matches the measured commit or the pull request's head
commit, so `--commit $(git rev-parse HEAD)` on a PR branch finds the PR's report;
--id names a report; --event push|pull_request narrows --pr / --commit.

The baseline is the one the server recorded for the head at upload, the one the PR
comment compared. The judging policy is the one the head report carried when it was
uploaded, named by `head.policy_path` and `head.policy_url`, so the answer for a
stored report does not change with a later edit of the policy. No other policy ever stands in: a head report
whose policy can no longer be read is answered as `unreadable` naming `head`, with
the policy file in `error`.
Every value is a string formatted as the PR comment shows it (\"2.06 ms\", \"3.1 KB\",
\"20\"), and its `unit` says how to read it. `change_percent` stays a number of
percent and is the exact figure to reason about: rounding can make `base` and `head`
read the same while it is not zero. A cell names its `column`, and `family.metrics`
names the columns the policy judges.

Each section is one metric family the policy lists; a family the policy leaves out is
not in the body at all. A judged family decides the verdict; an unjudged one is still
assessed (outcomes and crossed cells) but never counts. By default each section lists
only the rows the PR comment lists: regressions, improvements, added and removed rows
of judged families. Each of those rows carries only the cells that explain it: the
ones that crossed the family's bar (the family's metrics for an added or removed row)
and the context columns, which have a `role` (calls, % total). A column whose value
did not parse has no cell and is named in the row's `unreadable`. A section without
rows is still sent, with its `family` and `counts` but without its totals. The
verdict, the run totals and each family's `counts` are always complete, and `rows` in
the body says which filter applied. --full lists every row of every section, unjudged
families included, every cell of every row, the section totals and `columns`, the
legend that says which way is worse for each column. --advisory is the step between
the two: the default cut, plus the regressions, improvements, added and removed rows
of unjudged families, so a nonzero count in an unjudged family's `counts` can be
inspected without --full. Those rows never count toward the verdict; `family.judged`
of their section says so.

--select JSON keeps only the rows and truncation omissions of named entities, as
`report --select` names them:
'{\"functions\":{\"keys\":[\"app::hot\"]},\"io\":{\"keys\":[\"sock\"]}}' (an io label
keeps every op row of it). Alone it lists every row of those entities, as --full
does (unchanged and ignored rows included); with --advisory only their findings. A
section left without rows or omissions is dropped; a section the policy leaves out
is never added. The verdict, the budgets, the totals, the coverages, the counts and
the exit code describe the whole comparison, the same as without --select.

Inside a section, a key that is unset or an empty list is
left out: `mode`, `totals`, `columns`, `omitted_from_base`, `omitted_from_head`, a
row's `location` and `unreadable`, a cell's `role` and `crossed`. A cell's `base`,
`head` and `change_percent` are always there, null when a side is absent, and so is
every key outside the sections (`base`, `budgets`, the run totals).

Budgets are the policy's absolute bounds on named entities (`[[functions.budgets]]`,
`[[sql.budgets]]`, ...). They are judged on the head report alone, so a report without
a baseline (a push to main, a pull request whose base has no report yet) still gets an
answer. `budgets.rules` counts the policy's rules and `budgets.broken` the broken
checks; `budgets.findings` lists the broken ones, every check with --full, and
`budgets.notes` says what could not be checked. `budgets` is null when the head report
does not parse.

The top-level `verdict` covers the comparison and the budgets together: `judged` says
whether anything was judged, `regressed` whether a judged family regressed or a budget
is broken.

Exit codes:
  0  `verdict.judged` and not `verdict.regressed`
  1  everything else: something failed, nothing was judged, or any error (unknown
     report, bad argument value, auth, network, unparseable body)
  2  usage error
So 0 means exactly \"judged and fine\":
  0  compared, no regression, budgets hold (or the policy has none)
  1  compared, a regression (see `sections[].rows` whose `outcome` is `regression`
     and their `crossed` cells)
  1  compared, no regression, a budget broken
  0  no baseline or an unreadable one, budgets hold
  1  no baseline, a budget broken
  1  no baseline, a policy without budgets (nothing judged)
  1  head unreadable (nothing judged)
Which kind of 1 it was is in the output: a server answer prints the body on stdout
with stderr empty, as on exit 0; an error prints one JSON document on stderr with
stdout empty, as for every command. Reading needs only access to the repository.
Poll with `report` to wait for CI's upload. Same token and base URL environment as `auth`."
    )]
    Diff(DiffArgs),

    #[command(
        about = "Check that a policy file of this repository parses as a policy",
        long_about = "Check that a policy file of this repository parses as a policy
(POST /api/v1/policy/validate). One file is checked per run.

A policy is a TOML file in the repository. A run reads it and sends it inside its
report, and the server judges the report under it, so a pull request is judged under
its own policy. A run picks the first of:
  1. the file HOTPATH_POLICY_PATH names (relative to the working directory, or
     absolute; it must be inside the repository)
  2. hotpath/<benchmark>-policy.toml
  3. hotpath/policy.toml
and without any of them the upload is refused (`init` writes a starting file). A key
a file leaves out takes the release's default. The repository is the git repository
the working directory is in.

Without arguments the file a run without a benchmark policy of its own picks is
checked: HOTPATH_POLICY_PATH if set, else hotpath/policy.toml. --benchmark NAME checks
the file a run of that benchmark picks, by the order above, so each
hotpath/<benchmark>-policy.toml is checked with its own --benchmark. --file PATH checks
that file wherever it is, `-` reads stdin.

The output is one JSON object: `path` (relative to the repository root, as given for
--file), `valid`, and `problems`, empty when the file is valid. A problem has a
`message` and the `line` it points at, null when it concerns the whole file. A TOML
syntax error stops parsing, so it is reported alone; otherwise every structural and
range problem of the file comes in one answer. The file is not parsed here, the
server judges it. A file that is unreadable, not valid UTF-8, blank, larger
than 65536 bytes or outside the repository is reported as a problem of that file
without a request, and a run that finds such a file does not upload its report.
An upload whose policy the server finds not valid is refused, so check each file
before it reaches the default branch.

Exit codes:
  0  the file is valid
  1  the file is not valid, no policy file was found, or any error (auth, network,
     unparseable body)
  2  usage error
A file that is not valid is an answer: the object is printed on stdout with stderr
empty. An error prints one JSON document on stderr with stdout empty, as for every
command. The route is public: no token is needed and HOTPATH_API_TOKEN is never sent.
Same base URL environment as `auth`."
    )]
    ValidatePolicy(ValidatePolicyArgs),

    #[command(
        about = "Write the default policy as this repository's policy file (GET /api/v1/policy/default)",
        long_about = "Write the default policy as this repository's policy file (GET /api/v1/policy/default).

Every uploaded report must carry a policy file of its repository, and a key the file
leaves out takes the release's default. This command fetches those defaults, comments
included, and writes them as hotpath/policy.toml, the file every benchmark without
its own policy is judged under, or with --benchmark NAME as hotpath/NAME-policy.toml.
The document comes from the server, so it always matches the release that judges.
Committed unchanged, it judges exactly as a file that sets no key; edit it, then check
it with `validate-policy`. HOTPATH_POLICY_PATH, when set at run time, still wins over
both files.

The repository is the git repository the working directory is in. An existing file is
never replaced without --force, and a symlink never; both are refused before any
request. The output is one JSON object: `path` (relative to the repository root) and
`replaced`, whether an existing file was replaced.

Exit codes:
  0  the file was written
  1  the file exists (without --force), no git repository, or any error (auth,
     network, unparseable body)
  2  usage error
The route is public: no token is needed and HOTPATH_API_TOKEN is never sent. Same base
URL environment as `auth`."
    )]
    Init(InitArgs),
}

impl CloudArgs {
    /// Runs the command; every failure is one JSON document on stderr and exit 1.
    /// Each command validates its arguments before it builds the client, so a
    /// bad argument is reported without a token and never costs a request.
    pub(crate) fn run(self) -> ExitCode {
        let CloudArgs {
            cmd,
            pretty,
            output,
        } = self;
        let output = Output {
            pretty,
            file: output,
        };
        match Self::execute(cmd, &output) {
            Ok(code) => code,
            Err(error) => {
                output.emit_error(&error);
                ExitCode::FAILURE
            }
        }
    }

    fn execute(cmd: CloudCommand, output: &Output) -> Result<ExitCode, CliError> {
        match cmd {
            CloudCommand::Auth => auth::run(output),
            CloudCommand::Repos => repos::run(output),
            CloudCommand::Benchmarks { repo } => benchmarks::run(output, &repo),
            CloudCommand::Report(args) => report::run(output, args),
            CloudCommand::Reports(args) => reports::run(output, *args),
            CloudCommand::Diff(args) => diff::run(output, args),
            CloudCommand::ValidatePolicy(args) => policy::validate(output, args),
            CloudCommand::Init(args) => init::run(output, args),
        }
    }
}

//! `hotpath cloud reports --repo owner/name [filters]`: one page of the
//! `GET /api/v1/repos/{owner}/{name}/reports` listing, printed as received.
//! Every filter is optional and they combine with AND; values are checked
//! here by the server's rules and percent-encoded into the query. A page with
//! regressed or failed reports is still a successful listing (exit 0).

use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::{ArgAction, Args, ValueEnum};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime};

use crate::cmd::cloud::api::{with_query, CliError, Client, Output};
use crate::cmd::cloud::repo;
use crate::cmd::cloud::select::{SelectArg, MAX_SELECTED_LIMIT};
use crate::cmd::cloud::selector::{validate_commit, validate_pr, Event};

/// The server's page size bound; it applies its own default when `--limit`
/// is left out.
const MAX_LIMIT: u64 = 50;
/// The server's longest `branch=`, in bytes.
const MAX_BRANCH_BYTES: usize = 255;
/// The server's `metadata=` bounds: at most this many pairs, each key and
/// value at most as long as an upload's `user_metadata` allows, in bytes.
const MAX_METADATA_PAIRS: usize = 8;
const MAX_METADATA_KEY_BYTES: usize = 64;
const MAX_METADATA_VALUE_BYTES: usize = 200;

#[derive(Args, Debug)]
pub(crate) struct ReportsArgs {
    #[arg(long, value_name = "OWNER/NAME", help = "The repository")]
    repo: String,

    #[arg(
        long,
        value_name = "NAME",
        help = "Only reports of this benchmark (default: every benchmark of the repository)"
    )]
    benchmark: Option<String>,

    #[arg(long, value_enum, help = "Only reports uploaded from this CI event")]
    event: Option<Event>,

    #[arg(long, value_name = "N", help = "Only reports of this pull request")]
    pr: Option<u64>,

    #[arg(
        long,
        value_name = "SHA",
        help = "Only reports measuring this full 40-character commit sha, or with it as PR head"
    )]
    commit: Option<String>,

    #[arg(
        long,
        value_name = "NAME",
        help = "Only reports measured on this branch (bare name, matches refs/heads/NAME)"
    )]
    branch: Option<String>,

    #[arg(
        long,
        value_name = "DATE",
        help = "Only reports uploaded at or after DATE (YYYY-MM-DD as UTC midnight, or RFC 3339)"
    )]
    since: Option<String>,

    #[arg(
        long,
        value_name = "DATE",
        help = "Only reports uploaded before DATE (YYYY-MM-DD as UTC midnight, or RFC 3339)"
    )]
    until: Option<String>,

    #[arg(long, value_enum, help = "Only reports with this stored verdict")]
    verdict: Option<VerdictFilter>,

    #[arg(
        long,
        value_name = "KEY=VALUE",
        action = ArgAction::Append,
        help = "Only reports whose user_metadata has KEY=VALUE (repeatable, all must match)"
    )]
    metadata: Vec<String>,

    #[command(flatten)]
    select: SelectArg,

    #[arg(
        long,
        value_name = "N",
        help = "Reports per page, 1 to 50 (server default 20); 1 to 20 with --select (default 10)"
    )]
    limit: Option<u64>,

    #[arg(
        long,
        value_name = "CURSOR",
        help = "Continue after the page whose `next_cursor` this is (same filters as that page)"
    )]
    cursor: Option<String>,
}

/// `--verdict`: which stored judgement a listed report has.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum VerdictFilter {
    /// Judged without regressions.
    #[value(name = "passed")]
    Passed,
    /// Judged with regressions, failed reports included.
    #[value(name = "regressed")]
    Regressed,
    /// Judged and failed.
    #[value(name = "failed")]
    Failed,
    /// No stored judgement.
    #[value(name = "not_judged")]
    NotJudged,
}

impl VerdictFilter {
    fn as_str(self) -> &'static str {
        match self {
            VerdictFilter::Passed => "passed",
            VerdictFilter::Regressed => "regressed",
            VerdictFilter::Failed => "failed",
            VerdictFilter::NotJudged => "not_judged",
        }
    }
}

pub(crate) fn run(output: &Output, args: ReportsArgs) -> Result<ExitCode, CliError> {
    let path = request_path(&args)?;
    let reports = Client::from_env()?.get_raw(&path)?;
    output.emit_raw(&reports)?;
    Ok(ExitCode::SUCCESS)
}

/// The request path and query for validated `args`, or the error naming the
/// first bad argument. Only the filters given are sent.
fn request_path(args: &ReportsArgs) -> Result<String, CliError> {
    let repo = repo::validate(&args.repo)?;
    let benchmark = args
        .benchmark
        .as_deref()
        .map(repo::validate_benchmark)
        .transpose()?;
    let pr = args
        .pr
        .map(validate_pr)
        .transpose()?
        .map(|pr| pr.to_string());
    let commit = args.commit.as_deref().map(validate_commit).transpose()?;
    let branch = args.branch.as_deref().map(validate_branch).transpose()?;
    let since = args
        .since
        .as_deref()
        .map(|raw| time_bound("since", raw))
        .transpose()?;
    let until = args
        .until
        .as_deref()
        .map(|raw| time_bound("until", raw))
        .transpose()?;
    if let (Some(since), Some(until)) = (since, until) {
        if since >= until {
            return Err(CliError::client(
                "invalid --since / --until: --since must be before --until.",
            ));
        }
    }
    let metadata = metadata_filter(&args.metadata)?;
    let select = args.select.validate()?;
    let limit = args
        .limit
        .map(|limit| validate_limit(limit, select.is_some()))
        .transpose()?;

    // The dates go as given, so their offsets reach the server unchanged.
    let params: Vec<(&str, &str)> = [
        ("benchmark", benchmark),
        ("event", args.event.map(Event::as_str)),
        ("pr", pr.as_deref()),
        ("commit", commit.as_deref()),
        ("branch", branch),
        ("since", args.since.as_deref()),
        ("until", args.until.as_deref()),
        ("verdict", args.verdict.map(VerdictFilter::as_str)),
        ("metadata", metadata.as_deref()),
        ("select", select),
        ("limit", limit.as_deref()),
        ("cursor", args.cursor.as_deref()),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key, value?)))
    .collect();
    Ok(with_query(
        &format!("/api/v1/repos/{repo}/reports"),
        &params,
    ))
}

/// A `--branch` value by the server's rule: a bare branch name, not a ref.
fn validate_branch(branch: &str) -> Result<&str, CliError> {
    if branch.starts_with("refs/") {
        return Err(CliError::client(format!(
            "invalid --branch `{branch}`: expected the bare branch name (main), not a ref."
        )));
    }
    if branch.is_empty() || branch.len() > MAX_BRANCH_BYTES || branch.chars().any(char::is_control)
    {
        return Err(CliError::client(format!(
            "invalid --branch {branch:?}: expected a branch name of 1 to {MAX_BRANCH_BYTES} bytes without control characters."
        )));
    }
    Ok(branch)
}

/// The `--metadata KEY=VALUE` flags as the server's `metadata=` JSON
/// object, or `None` without any. Each flag splits on its first `=`, so a
/// value may hold `=` or `,`; keys and values are matched as given, so
/// nothing is trimmed.
fn metadata_filter(flags: &[String]) -> Result<Option<String>, CliError> {
    if flags.is_empty() {
        return Ok(None);
    }
    if flags.len() > MAX_METADATA_PAIRS {
        return Err(CliError::client(format!(
            "invalid --metadata: expected at most {MAX_METADATA_PAIRS} KEY=VALUE pairs, got {}.",
            flags.len()
        )));
    }
    let mut pairs = BTreeMap::new();
    for flag in flags {
        let bad = |expected: String| {
            CliError::client(format!("invalid --metadata `{flag}`: expected {expected}."))
        };
        let Some((key, value)) = flag.split_once('=') else {
            return Err(bad("KEY=VALUE".into()));
        };
        if key.is_empty() || key.len() > MAX_METADATA_KEY_BYTES {
            return Err(bad(format!(
                "a KEY of 1 to {MAX_METADATA_KEY_BYTES} bytes before the `=`"
            )));
        }
        if value.len() > MAX_METADATA_VALUE_BYTES {
            return Err(bad(format!(
                "a VALUE of at most {MAX_METADATA_VALUE_BYTES} bytes"
            )));
        }
        if pairs.insert(key, value).is_some() {
            return Err(bad(format!("each KEY once, `{key}` is given twice")));
        }
    }
    serde_json::to_string(&pairs)
        .map(Some)
        .map_err(|e| CliError::client(format!("invalid --metadata: {e}.")))
}

/// A `--since` / `--until` value: RFC 3339, or `YYYY-MM-DD` read as UTC midnight.
fn time_bound(flag: &str, raw: &str) -> Result<OffsetDateTime, CliError> {
    if let Ok(at) = OffsetDateTime::parse(raw, &Rfc3339) {
        return Ok(at);
    }
    Date::parse(raw, format_description!("[year]-[month]-[day]"))
        .map(|day| day.midnight().assume_utc())
        .map_err(|_| {
            CliError::client(format!(
                "invalid --{flag} `{raw}`: expected a YYYY-MM-DD date or an RFC 3339 time, e.g. 2026-11-01 or 2026-11-01T10:30:00Z."
            ))
        })
}

/// `--limit` by the server's bound: lower with `--select`, each listed
/// payload being read whole.
fn validate_limit(limit: u64, selected: bool) -> Result<String, CliError> {
    let (max, with) = if selected {
        (MAX_SELECTED_LIMIT, " with --select")
    } else {
        (MAX_LIMIT, "")
    };
    if (1..=max).contains(&limit) {
        Ok(limit.to_string())
    } else {
        Err(CliError::client(format!(
            "invalid --limit `{limit}`: expected a number of reports from 1 to {max}{with}."
        )))
    }
}

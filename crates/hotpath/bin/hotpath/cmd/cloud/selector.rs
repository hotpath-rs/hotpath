//! The head selector `report` and `diff` share: exactly one of `--pr`,
//! `--commit` and `--id`, optionally narrowed by `--event`. Validated values
//! are URL-safe as they are and go into the path and query unencoded.

use std::num::NonZeroU64;

use clap::{ArgGroup, Args, ValueEnum};

use crate::cmd::cloud::api::CliError;

/// Which report a command acts on, as parsed by clap; `validate` checks the
/// values.
#[derive(Args, Debug)]
#[command(group(
    ArgGroup::new("selector")
        .args(["pr", "commit", "id"])
        .required(true)
        .multiple(false)
))]
pub(crate) struct ReportSelector {
    #[arg(long, value_name = "N", help = "Newest report of this pull request")]
    pr: Option<u64>,

    #[arg(
        long,
        value_name = "SHA",
        help = "Newest report measuring this full 40-character commit sha, or with it as PR head"
    )]
    commit: Option<String>,

    #[arg(long, value_name = "ID", help = "The report with this id (a uuid)")]
    id: Option<String>,

    #[arg(
        long,
        value_enum,
        conflicts_with = "id",
        help = "Only reports of this event (with --pr or --commit)"
    )]
    pub(crate) event: Option<Event>,
}

/// The CI event a report was uploaded from, as `--event` narrows it.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub(crate) enum Event {
    #[value(name = "push")]
    Push,
    #[value(name = "pull_request")]
    PullRequest,
}

impl Event {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Event::Push => "push",
            Event::PullRequest => "pull_request",
        }
    }
}

/// Which report, after validation: a value here is safe in a URL as is.
pub(crate) enum Selector {
    /// Newest report of this pull request.
    Pr(NonZeroU64),
    /// Newest report whose measured commit or PR head is this lowercase sha.
    Commit(String),
    /// This report.
    Id(String),
}

impl ReportSelector {
    pub(crate) fn validate(&self) -> Result<Selector, CliError> {
        // The required, single-choice `selector` group guarantees exactly one.
        if let Some(pr) = self.pr {
            return validate_pr(pr).map(Selector::Pr);
        }
        if let Some(commit) = &self.commit {
            return validate_commit(commit).map(Selector::Commit);
        }
        let id = self.id.as_deref().unwrap_or_default();
        if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
            return Err(CliError::client(format!(
                "invalid --id `{id}`: expected a report id (a 36-character uuid)."
            )));
        }
        Ok(Selector::Id(id.to_string()))
    }
}

/// A `--pr` value: a pull request number the server stores (1 to `i32::MAX`).
pub(crate) fn validate_pr(pr: u64) -> Result<NonZeroU64, CliError> {
    NonZeroU64::new(pr)
        .filter(|pr| i32::try_from(pr.get()).is_ok())
        .ok_or_else(|| {
            CliError::client(format!(
                "invalid --pr `{pr}`: expected a pull request number from 1 to {}.",
                i32::MAX
            ))
        })
}

/// A `--commit` value: a full 40-character hex sha, lowercased as the server
/// stores it.
pub(crate) fn validate_commit(commit: &str) -> Result<String, CliError> {
    let sha = commit.to_ascii_lowercase();
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(CliError::client(format!(
            "invalid --commit `{commit}`: expected a full 40-character hex commit sha, e.g. --commit $(git rev-parse HEAD)."
        )));
    }
    Ok(sha)
}

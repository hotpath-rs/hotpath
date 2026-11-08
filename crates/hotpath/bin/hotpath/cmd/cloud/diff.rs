//! `hotpath cloud diff`: one stored report against its upload-time baseline,
//! judged under the benchmark's current policy, as JSON. The head is picked
//! like `report` (`selector.rs`). Without flags only the PR comment's findings
//! are sent; `--full` (`rows=all`) asks for every row and cell, `--advisory`
//! also lists the unjudged families' findings. `--select` (`select.rs`) keeps
//! only the named entities' rows; alone it means every row of them (the
//! server's `rows=all` default), with `--advisory` only their findings.
//!
//! Exit 0 means exactly "judged and nothing failed"
//! (`verdict.judged && !verdict.regressed`), exit 1 is everything else. A
//! missing or unreadable baseline does not fail a report whose budgets hold.
//! The exit code is read from the body's top-level `verdict` (`DiffVerdict`),
//! never from the HTTP status, which is 200 for every result.

use std::process::ExitCode;

use clap::Args;
use hotpath::json::cloud_api::Verdict;
use serde::Deserialize;

use crate::cmd::cloud::api::{with_query, CliError, Client, Output};
use crate::cmd::cloud::repo;
use crate::cmd::cloud::select::SelectArg;
use crate::cmd::cloud::selector::{ReportSelector, Selector};

#[derive(Args, Debug)]
pub(crate) struct DiffArgs {
    #[arg(long, value_name = "OWNER/NAME", help = "The repository")]
    repo: String,

    #[arg(long, value_name = "NAME", help = "The benchmark (series)")]
    benchmark: String,

    #[command(flatten)]
    selector: ReportSelector,

    #[arg(
        long,
        help = "Every row and every cell of every section, with the column legend, not only the PR comment's findings"
    )]
    full: bool,

    #[arg(
        long,
        conflicts_with = "full",
        help = "The findings of unjudged families too, which never count toward the verdict"
    )]
    advisory: bool,

    #[command(flatten)]
    select: SelectArg,
}

pub(crate) fn run(output: &Output, args: DiffArgs) -> Result<ExitCode, CliError> {
    let path = request_path(&args)?;
    let client = Client::from_env()?;
    let diff = client.get_raw(&path)?;
    // Parsed before anything is printed: a body without a verdict prints
    // nothing on stdout.
    let DiffVerdict { verdict } = diff.parse()?;
    output.emit_raw(&diff)?;
    Ok(exit_code(&verdict))
}

/// The part of a diff body the exit code reads; serde ignores the rest.
#[derive(Deserialize)]
struct DiffVerdict {
    verdict: Verdict,
}

fn exit_code(verdict: &Verdict) -> ExitCode {
    if verdict.judged && !verdict.regressed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// The request path and query for validated `args`, or the error naming the
/// first bad argument.
fn request_path(args: &DiffArgs) -> Result<String, CliError> {
    let repo = repo::validate(&args.repo)?;
    let benchmark = repo::validate_benchmark(&args.benchmark)?;
    let selector = args.selector.validate()?;
    let select = args.select.validate()?;

    let pr;
    let head = match &selector {
        Selector::Pr(n) => {
            pr = n.to_string();
            ("pr", pr.as_str())
        }
        Selector::Commit(sha) => ("commit", sha.as_str()),
        Selector::Id(id) => ("head", id.as_str()),
    };
    // Without `rows=` the server sends findings only, or with `select=` every
    // row of the named entities. clap rejects `--event` together with `--id`,
    // and `--full` together with `--advisory`.
    let rows = if args.full {
        Some("all")
    } else if args.advisory {
        Some("advisory")
    } else {
        None
    };
    let params: Vec<(&str, &str)> = [
        Some(head),
        args.selector.event.map(|event| ("event", event.as_str())),
        rows.map(|rows| ("rows", rows)),
        select.map(|select| ("select", select)),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(with_query(
        &format!("/api/v1/repos/{repo}/benchmarks/{benchmark}/diff"),
        &params,
    ))
}

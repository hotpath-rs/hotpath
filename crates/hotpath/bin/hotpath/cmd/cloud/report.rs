//! `hotpath cloud report`: one stored report, selected by pull request, commit
//! or id (`selector.rs`), with or without its payload. Also the agent loop's
//! polling primitive: "has CI uploaded a report for my commit yet" is exit 0
//! here instead of exit 1 with the server's `not_found`.

use std::process::ExitCode;

use clap::Args;

use crate::cmd::cloud::api::{with_query, CliError, Client, Output};
use crate::cmd::cloud::repo;
use crate::cmd::cloud::select::SelectArg;
use crate::cmd::cloud::selector::{ReportSelector, Selector};

#[derive(Args, Debug)]
pub(crate) struct ReportArgs {
    #[arg(long, value_name = "OWNER/NAME", help = "The repository")]
    repo: String,

    #[arg(long, value_name = "NAME", help = "The benchmark (series)")]
    benchmark: String,

    #[command(flatten)]
    selector: ReportSelector,

    #[arg(long, help = "Leave out the report payload (summary only)")]
    no_payload: bool,

    #[command(flatten)]
    select: SelectArg,
}

pub(crate) fn run(output: &Output, args: ReportArgs) -> Result<ExitCode, CliError> {
    let path = request_path(&args)?;
    // The request (`payload=false`), not the client, decides the body's shape.
    let report = Client::from_env()?.get_raw(&path)?;
    output.emit_raw(&report)?;
    Ok(ExitCode::SUCCESS)
}

/// The request path and query for validated `args`, or the error naming the
/// first bad argument.
fn request_path(args: &ReportArgs) -> Result<String, CliError> {
    let repo = repo::validate(&args.repo)?;
    let benchmark = repo::validate_benchmark(&args.benchmark)?;
    let selector = args.selector.validate()?;
    let select = args.select.validate()?;

    let base = format!("/api/v1/repos/{repo}/benchmarks/{benchmark}/reports");
    let pr;
    let (path, at) = match &selector {
        Selector::Pr(n) => {
            pr = n.to_string();
            (format!("{base}/latest"), Some(("pr", pr.as_str())))
        }
        Selector::Commit(sha) => (format!("{base}/latest"), Some(("commit", sha.as_str()))),
        Selector::Id(id) => (format!("{base}/{id}"), None),
    };
    // clap rejects `--event` together with `--id`.
    let params: Vec<(&str, &str)> = [
        at,
        args.selector.event.map(|event| ("event", event.as_str())),
        args.no_payload.then_some(("payload", "false")),
        select.map(|select| ("select", select)),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(with_query(&path, &params))
}

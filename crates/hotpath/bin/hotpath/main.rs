mod cmd;

#[cfg(any(feature = "tui", feature = "cloud"))]
use clap::{Parser, Subcommand};
#[cfg(feature = "cloud")]
use cmd::cloud::CloudArgs;
#[cfg(feature = "tui")]
use cmd::console::ConsoleArgs;
#[cfg(any(feature = "tui", feature = "cloud"))]
use std::process::ExitCode;

/// Placeholder for `hotpath cloud` in a binary built without the `cloud`
/// feature: keeps the command visible in `--help` and turns an attempt to
/// run it into a hint instead of clap's "unrecognized subcommand".
#[cfg(all(any(feature = "tui", feature = "cloud"), not(feature = "cloud")))]
#[derive(Parser, Debug)]
pub(crate) struct CloudUnavailableArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    pub(crate) rest: Vec<String>,
}

#[cfg(any(feature = "tui", feature = "cloud"))]
#[derive(Subcommand, Debug)]
pub(crate) enum HPSubcommand {
    #[cfg(feature = "tui")]
    #[command(about = "Launch TUI console to monitor profiling metrics in real-time")]
    Console(ConsoleArgs),
    #[cfg(feature = "cloud")]
    #[command(about = "Query hotpath.rs: authentication status and cloud reports")]
    Cloud(CloudArgs),
    #[cfg(not(feature = "cloud"))]
    #[command(about = "Query hotpath.rs (requires building with the 'cloud' feature)")]
    Cloud(CloudUnavailableArgs),
}

#[cfg(any(feature = "tui", feature = "cloud"))]
#[derive(Parser, Debug)]
#[command(
    version,
    about,
    long_about = "hotpath CLI: automatically profile Rust programs on each Pull Request

https://github.com/hotpath-rs/hotpath",
    args_conflicts_with_subcommands = true
)]
pub(crate) struct HPArgs {
    #[command(subcommand)]
    pub(crate) cmd: Option<HPSubcommand>,

    #[cfg(feature = "tui")]
    #[command(flatten)]
    pub(crate) console_args: ConsoleArgs,
}

#[cfg(not(feature = "cloud"))]
pub(crate) const CLOUD_FEATURE_HINT: &str =
    "The 'cloud' command requires building with the 'cloud' feature: cargo install hotpath --features cloud";

#[cfg(any(feature = "tui", feature = "cloud"))]
#[hotpath::main(limit = 10)]
fn main() -> eyre::Result<ExitCode> {
    let root_args = HPArgs::parse();

    let code = match root_args.cmd {
        #[cfg(feature = "tui")]
        Some(HPSubcommand::Console(args)) => {
            args.run()?;
            ExitCode::SUCCESS
        }
        #[cfg(feature = "cloud")]
        Some(HPSubcommand::Cloud(args)) => args.run(),
        #[cfg(not(feature = "cloud"))]
        Some(HPSubcommand::Cloud(_)) => {
            eprintln!("{CLOUD_FEATURE_HINT}");
            ExitCode::FAILURE
        }
        #[cfg(feature = "tui")]
        None => {
            root_args.console_args.run()?;
            ExitCode::SUCCESS
        }
        #[cfg(not(feature = "tui"))]
        None => {
            use clap::CommandFactory;
            HPArgs::command().print_help()?;
            ExitCode::FAILURE
        }
    };

    Ok(code)
}

#[cfg(not(any(feature = "tui", feature = "cloud")))]
fn main() -> std::process::ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("cloud") => eprintln!("{CLOUD_FEATURE_HINT}"),
        _ => eprintln!(
            "hotpath CLI

The 'console' command requires building with the 'tui' feature.
The 'cloud' command requires building with the 'cloud' feature."
        ),
    }
    std::process::ExitCode::FAILURE
}

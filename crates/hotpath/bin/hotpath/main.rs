mod cmd;

#[cfg(any(feature = "tui", feature = "cloud"))]
use clap::{Parser, Subcommand};
#[cfg(feature = "cloud")]
use cmd::cloud::CloudArgs;
#[cfg(feature = "tui")]
use cmd::console::ConsoleArgs;
#[cfg(any(feature = "tui", feature = "cloud"))]
use std::process::ExitCode;

#[cfg(any(feature = "tui", feature = "cloud"))]
#[derive(Parser, Debug)]
pub(crate) struct InitCliArgs {
    #[arg(long, help = "AI agent to launch: claude, codex or opencode")]
    pub(crate) agent: String,
}

#[cfg(any(feature = "tui", feature = "cloud"))]
impl InitCliArgs {
    fn run(&self, setup: cmd::init::Setup) -> eyre::Result<()> {
        let agent = cmd::init::Agent::from_arg(&self.agent).map_err(|e| eyre::eyre!(e))?;
        cmd::init::run(setup, agent).map_err(|e| eyre::eyre!(e))
    }
}

#[cfg(any(feature = "tui", feature = "cloud"))]
#[derive(Parser, Debug)]
pub(crate) struct InitCiCliArgs {
    #[command(flatten)]
    pub(crate) init: InitCliArgs,

    #[arg(
        long,
        help = "Also cover pull requests from forks, with a relay workflow that uploads their reports"
    )]
    pub(crate) forks: bool,
}

#[cfg(any(feature = "tui", feature = "cloud"))]
impl InitCiCliArgs {
    fn run(&self) -> eyre::Result<()> {
        self.init.run(if self.forks {
            cmd::init::Setup::CiForks
        } else {
            cmd::init::Setup::Ci
        })
    }
}

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
    #[command(about = "Configure hotpath in the current repo via an AI agent session")]
    Init(InitCliArgs),
    #[command(
        about = "Set up the hotpath Cloud CI integration in the current repo via an AI agent session"
    )]
    InitCi(InitCiCliArgs),
    #[command(
        about = "Set up a Grafana dashboard for the hotpath Prometheus metrics via an AI agent session"
    )]
    InitGrafana(InitCliArgs),
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

    match root_args.cmd {
        #[cfg(feature = "tui")]
        Some(HPSubcommand::Console(args)) => args.run()?,
        Some(HPSubcommand::Init(args)) => args.run(cmd::init::Setup::Profiling)?,
        Some(HPSubcommand::InitCi(args)) => args.run()?,
        Some(HPSubcommand::InitGrafana(args)) => args.run(cmd::init::Setup::Grafana)?,
        #[cfg(feature = "cloud")]
        Some(HPSubcommand::Cloud(args)) => return Ok(args.run()),
        #[cfg(not(feature = "cloud"))]
        Some(HPSubcommand::Cloud(_)) => {
            eprintln!("{CLOUD_FEATURE_HINT}");
            return Ok(ExitCode::FAILURE);
        }
        #[cfg(feature = "tui")]
        None => root_args.console_args.run()?,
        #[cfg(not(feature = "tui"))]
        None => {
            use clap::CommandFactory;
            HPArgs::command().print_help()?;
            return Ok(ExitCode::FAILURE);
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(not(any(feature = "tui", feature = "cloud")))]
fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some(command @ ("init" | "init-ci" | "init-grafana")) => {
            let result = parse_init_args(command, args)
                .and_then(|(setup, agent)| cmd::init::run(setup, agent));
            match result {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Some("cloud") => {
            eprintln!("{CLOUD_FEATURE_HINT}");
            std::process::ExitCode::FAILURE
        }
        _ => {
            eprintln!(
                "hotpath CLI

Usage: hotpath <COMMAND>

Commands:
  init --agent <claude|codex|opencode>     Configure hotpath in the current repo via an AI agent session
  init-ci --agent <claude|codex|opencode> [--forks]
                                           Set up the hotpath Cloud CI integration via an AI agent session
  init-grafana --agent <claude|codex|opencode>
                                           Set up a Grafana dashboard for the hotpath Prometheus metrics via an AI agent session

The 'console' command requires building with the 'tui' feature.
The 'cloud' command requires building with the 'cloud' feature."
            );
            std::process::ExitCode::FAILURE
        }
    }
}

/// `init`, `init-ci` and `init-grafana` arguments without clap: `--agent NAME` or
/// `--agent=NAME`, plus `--forks` for `init-ci`.
#[cfg(not(any(feature = "tui", feature = "cloud")))]
fn parse_init_args(
    command: &str,
    mut args: impl Iterator<Item = String>,
) -> Result<(cmd::init::Setup, cmd::init::Agent), String> {
    let usage = || match command {
        "init-ci" => "Usage: hotpath init-ci --agent <claude|codex|opencode> [--forks]".to_string(),
        _ => format!("Usage: hotpath {command} --agent <claude|codex|opencode>"),
    };
    let mut agent = None;
    let mut forks = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => agent = Some(args.next().ok_or_else(usage)?),
            "--forks" if command == "init-ci" => forks = true,
            _ => match arg.strip_prefix("--agent=") {
                Some(name) => agent = Some(name.to_string()),
                None => return Err(usage()),
            },
        }
    }
    let agent = cmd::init::Agent::from_arg(&agent.ok_or_else(usage)?)?;
    let setup = match (command, forks) {
        ("init-ci", true) => cmd::init::Setup::CiForks,
        ("init-ci", false) => cmd::init::Setup::Ci,
        ("init-grafana", _) => cmd::init::Setup::Grafana,
        _ => cmd::init::Setup::Profiling,
    };
    Ok((setup, agent))
}

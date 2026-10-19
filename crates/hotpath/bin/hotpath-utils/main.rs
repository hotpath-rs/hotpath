mod cmd;
use clap::{Parser, Subcommand};
use cmd::compare::CompareArgs;
use cmd::profile_pr::ProfilePrArgs;
use eyre::Result;

#[derive(Subcommand, Debug)]
pub enum HPSubcommand {
    #[command(about = "Profile a PR, compare with main branch, and post a GitHub comment")]
    ProfilePr(ProfilePrArgs),
    #[command(about = "Compare two JSON reports and print the diff")]
    Compare(CompareArgs),
}

#[derive(Parser, Debug)]
#[command(
    version,
    about,
    long_about = "hotpath CLI: automatically profile Rust programs on each Pull Request

https://github.com/hotpath-rs/hotpath"
)]
pub struct HPArgs {
    #[command(subcommand)]
    pub cmd: HPSubcommand,
}

fn main() -> Result<()> {
    let root_args = HPArgs::parse();

    match root_args.cmd {
        HPSubcommand::ProfilePr(args) => {
            args.run()?;
        }
        HPSubcommand::Compare(args) => {
            args.run()?;
        }
    }

    Ok(())
}

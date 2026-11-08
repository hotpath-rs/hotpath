//! `--precise`, shared by `report`, `reports` and `diff`: the server's
//! `precise=true`, measurements at the precision the uploaded report carries
//! instead of rounded for reading. Without it nothing is sent, so the server
//! default (rounded) applies.

use clap::Args;

#[derive(Args, Debug)]
pub(crate) struct PreciseArg {
    #[arg(
        long,
        help = "Unrounded measurements: durations in ns, sizes in B, changes unrounded \
                (default: rounded for reading)"
    )]
    precise: bool,
}

impl PreciseArg {
    /// The `precise=` value, or `None` without `--precise`.
    pub(crate) fn value(&self) -> Option<&'static str> {
        self.precise.then_some("true")
    }
}

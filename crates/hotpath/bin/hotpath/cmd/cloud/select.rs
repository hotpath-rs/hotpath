//! `--select JSON`, shared by `report`, `reports` and `diff`: the metrics of named
//! entities, sent as the server's `select=` JSON object. Only the shape is
//! checked here (a JSON object); resources, keys and the size bounds are the
//! server's to judge, its `bad_request` saying what to change.

use clap::Args;
use serde::de::IgnoredAny;

use crate::cmd::cloud::api::CliError;

/// Reports per page the server allows with `select=`: each payload is read
/// whole. It applies its own default (10) when `--limit` is left out.
pub(crate) const MAX_SELECTED_LIMIT: u64 = 20;

#[derive(Args, Debug)]
pub(crate) struct SelectArg {
    #[arg(
        long,
        value_name = "JSON",
        help = "Add `metrics`: every measurement of these entities, as a JSON object of \
                resources and their exact keys ({\"functions\":{\"keys\":[\"app::hot\"]}})"
    )]
    select: Option<String>,
}

impl SelectArg {
    /// The `select=` value as given, or `None` without `--select`. The text
    /// is sent unchanged, never re-serialized: a JSON map would keep only the
    /// last of a repeated resource, which the server refuses.
    pub(crate) fn validate(&self) -> Result<Option<&str>, CliError> {
        let Some(raw) = self.select.as_deref() else {
            return Ok(None);
        };
        let bad = |expected: &str| {
            CliError::client(format!("invalid --select `{raw}`: expected {expected}."))
        };
        if !raw.trim_start().starts_with('{') {
            return Err(bad("a JSON object of resources"));
        }
        serde_json::from_str::<IgnoredAny>(raw)
            .map_err(|e| bad(&format!("a JSON object of resources ({e})")))?;
        Ok(Some(raw.trim()))
    }
}

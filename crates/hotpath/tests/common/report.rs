use hotpath::json::JsonReport;

/// Every finalized report is also checked to carry no entry ids.
#[track_caller]
pub(crate) fn parse_report(stdout: &str) -> JsonReport {
    let value = crate::common::first_json_value(stdout);
    assert_no_entry_ids(&value);
    serde_json::from_value(value).expect("Failed to parse JSON report")
}

/// `section[index]` of every entry carrying an `id` key. Entry ids address
/// entries within one session only, so a finalized report has none.
pub(crate) fn entry_ids(report: &serde_json::Value) -> Vec<String> {
    let Some(sections) = report.as_object() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for (section, value) in sections {
        for key in ["data", "entries"] {
            let Some(entries) = value.get(key).and_then(|v| v.as_array()) else {
                continue;
            };
            for (i, entry) in entries.iter().enumerate() {
                if entry.get("id").is_some() {
                    found.push(format!("{section}[{i}]"));
                }
            }
        }
    }
    found
}

#[track_caller]
pub(crate) fn assert_no_entry_ids(report: &serde_json::Value) {
    let found = entry_ids(report);
    assert!(
        found.is_empty(),
        "entry ids in a finalized report: {found:?}"
    );
}

impl crate::common::example::Example {
    /// Runs to completion, asserts a successful exit and parses the JSON
    /// report from stdout.
    #[track_caller]
    pub(crate) fn report(self) -> JsonReport {
        parse_report(&self.stdout())
    }
}

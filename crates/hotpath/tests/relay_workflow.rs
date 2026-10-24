#[cfg(all(test, feature = "hotpath"))]
mod tests {
    //! The relay workflow's size check on the body it posts: its limit is
    //! the client contract's, and it runs at the exact boundary.
    //!
    //! cargo test --features hotpath --test relay_workflow

    use std::path::PathBuf;
    use std::process::Command;

    use hotpath::json::cloud_api::MAX_REPORT_BYTES;

    const WORKFLOW: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/hotpath-relay.yml"
    );

    /// The check's shell lines, from `max_bytes=` to its closing `fi`.
    fn size_check() -> String {
        let workflow = std::fs::read_to_string(WORKFLOW).expect("relay workflow");
        let lines: Vec<&str> = workflow
            .lines()
            .map(str::trim)
            .skip_while(|line| !line.starts_with("max_bytes="))
            .collect();
        let end = lines
            .iter()
            .position(|line| *line == "fi")
            .expect("size check ends with fi");
        lines[..=end].join("\n")
    }

    #[test]
    fn limit_matches_the_client_contract() {
        let script = size_check();
        assert!(
            script.starts_with(&format!("max_bytes={MAX_REPORT_BYTES}\n")),
            "{script}"
        );
    }

    #[test]
    fn body_at_the_limit_passes_and_one_byte_over_fails() {
        let dir: PathBuf =
            std::env::temp_dir().join(format!("hotpath-relay-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = size_check();

        for (len, passes) in [(MAX_REPORT_BYTES, true), (MAX_REPORT_BYTES + 1, false)] {
            std::fs::write(dir.join("upload.json"), vec![b' '; len]).unwrap();
            let output = Command::new("sh")
                .args(["-c", &script])
                .current_dir(&dir)
                .output()
                .expect("sh");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert_eq!(output.status.success(), passes, "{len} bytes: {stdout}");
            if !passes {
                assert!(
                    stdout.contains(&format!("::error::the report is {len} bytes")),
                    "{stdout}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

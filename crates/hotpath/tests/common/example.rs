use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::thread::sleep;
use std::time::Duration;

/// A `cargo run --example` invocation of a test crate, built up before it
/// is run to completion ([`Example::output`], [`Example::stdout`]) or left
/// running for endpoint polling ([`Example::spawn`]).
pub(crate) struct Example {
    cmd: Command,
    manifest_path: Option<String>,
    package: Option<String>,
    example: String,
    features: String,
    release: bool,
    args: Vec<String>,
}

impl Example {
    /// `cargo run -p <package> --example <example> --features hotpath`
    pub(crate) fn new(package: &str, example: &str) -> Self {
        let mut cmd = Command::new("cargo");
        // Many tests poll the metrics server, which is off on GitHub Actions
        // by default.
        cmd.env("HOTPATH_METRICS_SERVER_ON", "1");
        Self {
            cmd,
            manifest_path: None,
            package: Some(package.to_string()),
            example: example.to_string(),
            features: "hotpath".to_string(),
            release: false,
            args: Vec::new(),
        }
    }

    /// `cargo run --manifest-path <manifest_path> --example <example> --features hotpath`,
    /// for crates outside the workspace.
    pub(crate) fn in_manifest(manifest_path: &str, example: &str) -> Self {
        let mut this = Self::new("", example);
        this.package = None;
        this.manifest_path = Some(manifest_path.to_string());
        this
    }

    /// Adds `--manifest-path` next to `-p`, for runs with a foreign `current_dir`.
    pub(crate) fn manifest_path(mut self, manifest_path: &str) -> Self {
        self.manifest_path = Some(manifest_path.to_string());
        self
    }

    pub(crate) fn features(mut self, features: &str) -> Self {
        self.features = features.to_string();
        self
    }

    pub(crate) fn release(mut self) -> Self {
        self.release = true;
        self
    }

    /// Arguments passed to the example itself, after `--`.
    pub(crate) fn args(mut self, args: &[&str]) -> Self {
        self.args.extend(args.iter().map(|a| a.to_string()));
        self
    }

    pub(crate) fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.cmd.env(key, value);
        self
    }

    pub(crate) fn envs(mut self, envs: &[(&str, &str)]) -> Self {
        for (key, value) in envs {
            self.cmd.env(key, value);
        }
        self
    }

    pub(crate) fn env_remove(mut self, key: &str) -> Self {
        self.cmd.env_remove(key);
        self
    }

    pub(crate) fn current_dir(mut self, dir: &Path) -> Self {
        self.cmd.current_dir(dir);
        self
    }

    /// `HOTPATH_OUTPUT_FORMAT=json`
    pub(crate) fn json(self) -> Self {
        self.env("HOTPATH_OUTPUT_FORMAT", "json")
    }

    fn into_command(self) -> Command {
        let mut cmd = self.cmd;
        cmd.arg("run");
        if let Some(manifest_path) = &self.manifest_path {
            cmd.args(["--manifest-path", manifest_path]);
        }
        if let Some(package) = &self.package {
            cmd.args(["-p", package]);
        }
        cmd.args(["--example", &self.example, "--features", &self.features]);
        if self.release {
            cmd.arg("--release");
        }
        if !self.args.is_empty() {
            cmd.arg("--").args(&self.args);
        }
        cmd
    }

    /// Runs to completion without asserting on the exit status.
    pub(crate) fn output(self) -> Output {
        self.into_command()
            .output()
            .expect("Failed to execute command")
    }

    /// Runs to completion, asserts a successful exit and returns stdout.
    #[track_caller]
    pub(crate) fn stdout(self) -> String {
        let output = self.output();
        assert_success(&output);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Starts the example and leaves it running; it is killed when the
    /// returned guard drops.
    pub(crate) fn spawn(self) -> RunningExample {
        RunningExample {
            child: self
                .into_command()
                .spawn()
                .expect("Failed to spawn command"),
        }
    }
}

#[track_caller]
pub(crate) fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "Command failed with status: {}\nStdout:\n{}\nStderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// A spawned example, killed and reaped on drop so a failing assertion
/// never leaves it holding the test's port.
pub(crate) struct RunningExample {
    child: Child,
}

impl Drop for RunningExample {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// GETs `url` every `interval` until `ready` accepts the body. Once
/// `attempts` run out, panics if the last request failed and otherwise
/// returns the last body, leaving the precise assertion to the caller.
#[track_caller]
pub(crate) fn poll_endpoint(
    url: &str,
    attempts: u32,
    interval: Duration,
    ready: impl Fn(&str) -> bool,
) -> String {
    let mut body = String::new();
    let mut last_error = None;

    for _attempt in 0..attempts {
        sleep(interval);

        match ureq::get(url).call() {
            Ok(mut response) => {
                body = response
                    .body_mut()
                    .read_to_string()
                    .expect("Failed to read response body");
                last_error = None;
                if ready(&body) {
                    break;
                }
            }
            Err(e) => last_error = Some(e),
        }
    }

    if let Some(error) = last_error {
        panic!("GET {url} failed after {attempts} retries: {error}");
    }
    body
}

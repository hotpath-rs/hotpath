//! Valgrind probe and per-run scratch directories for Callgrind tests.

use std::path::PathBuf;
use std::process::Command;

/// Valgrind on x86_64 Linux. Locally the Valgrind tests skip without it;
/// on CI (`CI` set) a missing Valgrind fails them.
pub(crate) fn valgrind_available() -> bool {
    let supported = cfg!(all(target_arch = "x86_64", target_os = "linux"))
        && Command::new("valgrind")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
    if !supported {
        assert!(
            std::env::var_os("CI").is_none(),
            "valgrind is required on CI"
        );
        eprintln!("skipping: needs valgrind on x86_64 Linux");
    }
    supported
}

/// A fresh directory for one run's dumps and reports, removed on drop.
pub(crate) struct RunDir(pub(crate) PathBuf);

impl RunDir {
    pub(crate) fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hotpath-callgrind-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_string()
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

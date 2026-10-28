//! Touches about 200 MB, then execs itself into a profiled run, the way
//! `cargo run` execs into the binary it just built. On Linux the report's Max RSS
//! must cover only the exec'd program, not the ballast before it.
//!
//! Run with:
//!   HOTPATH_OUTPUT_FORMAT=json cargo run -p test-tokio-async --example rss_exec --features hotpath

const EXECED_ENV: &str = "RSS_EXEC_EXECED";

fn main() {
    if std::env::var_os(EXECED_ENV).is_none() {
        exec_after_ballast();
    }

    let _hotpath = hotpath::HotpathGuardBuilder::new("rss_exec::main")
        .sections(vec![hotpath::Section::Threads])
        .build();

    let work = vec![1u8; 1024 * 1024];
    std::hint::black_box(&work);
}

#[cfg(unix)]
fn exec_after_ballast() {
    use std::os::unix::process::CommandExt;

    const BALLAST_BYTES: usize = 200 * 1024 * 1024;
    let ballast = vec![1u8; BALLAST_BYTES];
    std::hint::black_box(&ballast);

    let exe = std::env::current_exe().expect("current exe");
    let err = std::process::Command::new(exe).env(EXECED_ENV, "1").exec();
    panic!("exec failed: {err}");
}

#[cfg(not(unix))]
fn exec_after_ballast() {}

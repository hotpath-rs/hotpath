// Exports the rustc version (report `meta.rustc`), the Cargo profile
// (Prometheus `hotpath_build_info`) and the alloc-mode nesting limit.
fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-env-changed=RUSTUP_TOOLCHAIN");
    println!("cargo:rerun-if-env-changed={ALLOC_MAX_DEPTH_VAR}");

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    // "rustc 1.89.0 (29483883e 2025-08-04)" -> "1.89.0"
    let version = std::process::Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|s| s.split_whitespace().nth(1).map(str::to_string))
        .unwrap_or_default();
    println!("cargo:rustc-env=HOTPATH_RUSTC_VERSION={version}");

    println!("cargo:rustc-env=HOTPATH_CARGO_PROFILE={}", cargo_profile());
    println!(
        "cargo:rustc-env={ALLOC_MAX_DEPTH_VAR}={}",
        alloc_max_depth()
    );
}

const ALLOC_MAX_DEPTH_VAR: &str = "HOTPATH_ALLOC_MAX_DEPTH";
const DEFAULT_ALLOC_MAX_DEPTH: usize = 64;
/// Each level costs 16 bytes of thread-local storage on every thread, so the
/// limit stays bounded.
const ALLOC_MAX_DEPTH_RANGE: std::ops::RangeInclusive<usize> = 1..=4096;

/// Maximum number of nested measured functions tracked in alloc mode. Read at
/// build time because it sizes the const-initialized thread-local frame stack.
fn alloc_max_depth() -> usize {
    let Ok(raw) = std::env::var(ALLOC_MAX_DEPTH_VAR) else {
        return DEFAULT_ALLOC_MAX_DEPTH;
    };
    match raw.trim().parse::<usize>() {
        Ok(depth) if ALLOC_MAX_DEPTH_RANGE.contains(&depth) => depth,
        _ => panic!(
            "Invalid {ALLOC_MAX_DEPTH_VAR} value: '{raw}'. Expected an integer in {}..={}.",
            ALLOC_MAX_DEPTH_RANGE.start(),
            ALLOC_MAX_DEPTH_RANGE.end()
        ),
    }
}

/// Cargo's `PROFILE` env var only distinguishes `debug` from `release`, so a
/// custom profile (`--profile profiling`) is read off `OUT_DIR`
/// (`target/[<triple>/]<profile>/build/<pkg>-<hash>/out`): the component
/// before `build` is the profile's directory name.
fn cargo_profile() -> String {
    let from_out_dir = std::env::var("OUT_DIR").ok().and_then(|out_dir| {
        let path = std::path::PathBuf::from(out_dir);
        let components: Vec<_> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let build_idx = components.iter().rposition(|c| c == "build")?;
        components.get(build_idx.checked_sub(1)?).cloned()
    });
    from_out_dir
        .or_else(|| std::env::var("PROFILE").ok())
        .unwrap_or_default()
}

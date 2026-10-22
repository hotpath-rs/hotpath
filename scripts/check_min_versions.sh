#!/usr/bin/env bash
#
# Resolves all workspace direct deps to their lowest semver-compatible versions
# and runs cargo check against hotpath. Backs up Cargo.toml + Cargo.lock and
# restores them on exit so the working tree stays clean.
#
# Requires: nightly toolchain (for -Z direct-minimal-versions) + stable toolchain.

set -euo pipefail

cd "$(dirname "$0")/.."

backups=()
cleanup() {
    for entry in "${backups[@]}"; do
        original="${entry%%::*}"
        backup="${entry##*::}"
        if [[ -f "$backup" ]]; then
            mv -f "$backup" "$original"
        elif [[ -f "$original" ]]; then
            rm -f "$original"
        fi
    done
}
trap cleanup EXIT

backup() {
    local file="$1"
    if [[ -f "$file" ]]; then
        cp -f "$file" "${file}.minver-bak"
        backups+=("${file}::${file}.minver-bak")
    else
        backups+=("${file}::${file}.minver-bak")
    fi
}

backup Cargo.toml
backup Cargo.lock
backup crates/hotpath/Cargo.toml
backup crates/hotpath-drain/Cargo.toml

python3 - <<'PY'
import re, pathlib

drop = (
    "crates/hotpath-meta",
    "crates/hotpath-macros-meta",
    "crates/hotpath-drain-meta",
)
p = pathlib.Path("Cargo.toml")
src = p.read_text()
for path in drop:
    src = re.sub(rf'\s*"{re.escape(path)}",', '', src)
src = re.sub(r'\s*"crates/test-[^"]*",', '', src)
src = re.sub(r'^hotpath-meta\s*=.*\n', '', src, flags=re.MULTILINE)
src = re.sub(r'^hotpath-macros-meta\s*=.*\n', '', src, flags=re.MULTILINE)
src = re.sub(r'^hotpath-drain-meta\s*=.*\n', '', src, flags=re.MULTILINE)
p.write_text(src)

p = pathlib.Path("crates/hotpath-drain/Cargo.toml")
src = p.read_text()
# Drops the optional dependency plus every `hotpath-meta` / `hotpath-*-meta` feature line.
src = re.sub(r'^hotpath-(\w+-)?meta\s*=.*\n', '', src, flags=re.MULTILINE)
p.write_text(src)

p = pathlib.Path("crates/hotpath/Cargo.toml")
src = p.read_text()
# Drops the optional dependency plus every `hotpath-meta` / `hotpath-*-meta` feature line.
src = re.sub(r'^hotpath-(\w+-)?meta\s*=.*\n', '', src, flags=re.MULTILINE)
src = re.sub(r'"hotpath-meta\??/[^"]+",?\s*', '', src)
src = re.sub(r'"dep:hotpath-meta",?\s*', '', src)
src = re.sub(r'"hotpath-drain\?/hotpath-[^"]+",?\s*', '', src)
extras = ("schemars", "rmcp", "axum", "tokio-util", "ureq", "reqwest",
          "reqwest-012", "reqwest-middleware-04", "reqwest-middleware-05",
          "async-trait", "http",
          "clap", "crossterm", "ratatui", "eyre", "tracing",
          "tracing-subscriber", "sqlx", "diesel", "libsqlite3-sys")
for dep in extras:
    src = re.sub(rf'^{re.escape(dep)}\s*=.*\n', '', src, flags=re.MULTILINE)
    src = re.sub(rf'"dep:{re.escape(dep)}",?\s*', '', src)
    src = re.sub(rf'"{re.escape(dep)}/[^"]*",?\s*', '', src)
src = re.sub(r'^demo\s*=.*\n', '', src, flags=re.MULTILINE)
src = re.sub(r'^"reqwest-0-1[23]"\s*=.*\n', '', src, flags=re.MULTILINE)
p.write_text(src)
PY

echo "==> cargo +nightly update -Z direct-minimal-versions"
cargo +nightly update -Z direct-minimal-versions

echo "==> cargo +stable check -p hotpath --features 'hotpath-alloc'"
cargo +stable check -p hotpath --features 'hotpath-alloc'

echo "OK: min-versions check passed"

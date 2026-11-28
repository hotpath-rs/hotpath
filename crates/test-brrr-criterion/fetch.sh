#!/usr/bin/env bash
# Downloads `src/main.rs` of brrr by Jon Gjengset (https://github.com/jonhoo/brrr,
# written on stream: https://youtu.be/tCY7p6dVAGE) at a pinned commit, checks
# its SHA-256 and applies `brrr.patch`, which adds the hotpath annotations and
# makes it a module. Upstream has no license, so its source is fetched rather
# than committed here.
set -euo pipefail
cd "$(dirname "$0")"

COMMIT=935fd02697990b36e00b57ee9fe5c2dc7a06be82
SHA256=f35214d6122254dc6c75acec5678c73811d1929e9ffc2067585167555ee62a48

mkdir -p upstream
curl -fsSL "https://raw.githubusercontent.com/jonhoo/brrr/$COMMIT/src/main.rs" -o upstream/main.rs
if command -v sha256sum > /dev/null; then
  echo "$SHA256  upstream/main.rs" | sha256sum -c --quiet -
else
  echo "$SHA256  upstream/main.rs" | shasum -a 256 -c --quiet -
fi
rm -f upstream/brrr.rs
patch -s -o upstream/brrr.rs upstream/main.rs brrr.patch

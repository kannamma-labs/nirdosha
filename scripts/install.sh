#!/bin/sh
# One-line installer for the nirdosha CLI (macOS / Linux) -- RETIRED.
#
# `crates/compiler` (the native `.nir` compiler this script installed
# a prebuilt binary of) is deprecated in favor of the v2 Rust dialect
# (`crates/cargo-nirdosha`/`crates/nirdosha-rt`/`crates/nirdosha-driver`
# -- see docs/nirdosha-rt-dialect.md). `.github/workflows/release.yml`
# no longer exists, so there is no release for this script to download
# from any more -- rather than silently 404ing, it refuses outright.
set -eu

echo "nirdosha: prebuilt binaries are no longer published -- crates/compiler is deprecated." >&2
echo "nirdosha: build the native compiler from source instead:" >&2
echo "nirdosha:   git clone https://github.com/kannamma-labs/nirdosha && cd nirdosha/crates/compiler && cargo build --release" >&2
echo "nirdosha: or use the v2 Rust dialect (the actively developed surface): see docs/nirdosha-rt-dialect.md" >&2
exit 1

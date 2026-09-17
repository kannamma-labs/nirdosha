# One-line installer for the nirdosha CLI (Windows) -- RETIRED.
#
# `crates/compiler` (the native `.nir` compiler this script installed
# a prebuilt binary of) is deprecated in favor of the v2 Rust dialect
# (`crates/cargo-nirdosha`/`crates/nirdosha-rt`/`crates/nirdosha-driver`
# -- see docs/nirdosha-rt-dialect.md). `.github/workflows/release.yml`
# no longer exists, so there is no release for this script to download
# from any more -- rather than silently failing on a missing asset, it
# refuses outright.

$ErrorActionPreference = "Stop"

Write-Error "nirdosha: prebuilt binaries are no longer published -- crates/compiler is deprecated.`nBuild the native compiler from source instead:`n  git clone https://github.com/kannamma-labs/nirdosha; cd nirdosha/crates/compiler; cargo build --release`nOr use the v2 Rust dialect (the actively developed surface): see docs/nirdosha-rt-dialect.md"
exit 1

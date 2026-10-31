#!/usr/bin/env bash
# The checks CI runs, in CI's order; CI calls this script, so run it before pushing.
# CI uses the stable toolchain and its lints are newer than a lagging nightly's:
# TOOLCHAIN=1.99.0 scripts/ci.sh runs them on a fixed one.
set -euo pipefail

cargo() { command cargo ${TOOLCHAIN:+"+$TOOLCHAIN"} "$@"; }
step() { printf '\n==> cargo %s\n' "$*"; cargo "$@"; }

step check --locked
# Native bindings, the CSS compiler, the scene core and the Caribou plugin stay warning-free (85d8966).
step clippy --locked --all-targets --features css-cli -- -D warnings
step clippy --locked --all-targets --no-default-features --features scene -- -D warnings
step clippy --locked --all-targets --no-default-features --features caribou -- -D warnings
# The default features include HashLink, whose tests the other lanes do not build.
step test --locked --lib
step test --locked --no-default-features --lib
step test --locked --no-default-features --features caribou --lib
step check --locked --no-default-features --features scene

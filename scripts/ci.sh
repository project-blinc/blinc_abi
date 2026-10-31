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
# Without `scene` only the CSS and layout core builds; `scene` is the text, display-list and tree code
# the default build tests, here without HashLink, which a runner has no libhl for (css_hl's tests need it).
step test --locked --no-default-features --lib
step test --locked --no-default-features --features scene --lib
step test --locked --no-default-features --features caribou --lib
step check --locked --no-default-features --features scene

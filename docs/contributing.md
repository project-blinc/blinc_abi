# Contributing

## Milestones and issue tracking

Track implementation work in git-bug with a concrete scope and acceptance checks.
Commit each tested milestone before starting the next one; keep fixes and changes
to separate subsystems in separate commits. Record the commit hash, checks run,
platform tested and remaining work in the issue. Close it only when its scope is
complete.

Use a follow-up issue for the next milestone. Keep verified commits available for
`git bisect`; use `git revert <commit>` to undo a committed change without rewriting
shared history. Check out a prior milestone in a separate worktree for comparisons
without disturbing current work.

```sh
git bug bug --format plain
git bug bug new --non-interactive --title 'Short title' --message 'Scope and acceptance checks'
git bug bug comment new ISSUE --non-interactive --message 'Commit, verification and remaining work'
git bug bug status close ISSUE
git push origin 'refs/bugs/*:refs/bugs/*' 'refs/identities/*:refs/identities/*'
```

Source commits and git-bug refs are separate: synchronize both when pushing a
milestone. Never include generated build output or host credentials in commits.

## Verification

```sh
cargo test --locked --no-default-features --lib
cargo clippy --locked --no-default-features --features scene -- -D warnings
cargo check --locked
cargo build --release --locked
```

Changes to the compatibility adapter must preserve its exported symbol names and
rendering behavior. Capture a scene with the existing consumer, then run the same
compiled scene against the extracted library in an isolated directory. Compare
pixels, including intermediate frames for reactive or layout changes. Preserve
the original consumer's source and library while making the comparison.

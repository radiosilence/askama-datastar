# askama_datastar — Askama templates as Datastar patches, for axum

# What CI checks, short of the audit and the download that checks the vendored
# script: format, lint with warnings as errors, tests, docs, the changelog, the
# workflows and the scripts.
check:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps
    python3 -m unittest scripts/test_changelog.py
    python3 scripts/changelog.py --lint
    actionlint
    shellcheck scripts/*.sh

# The checks that take seconds: Cargo.lock agrees with Cargo.toml, and the format.
fast:
    cargo metadata --locked --format-version 1 > /dev/null
    cargo fmt --all -- --check

# Format
fmt:
    cargo fmt --all

# Check Cargo.lock against RustSec advisories, with deny.toml's ignores
audit:
    cargo deny check advisories

# Install the git hooks (lefthook.yml)
hooks:
    lefthook install

# Write CHANGELOG.md from the fragments in changelog.d/. `--stdout` prints it instead.
changelog *args:
    python3 scripts/changelog.py {{args}}

# Vendor a Datastar release's script: `just datastar 1.0.6`. `just datastar --check`
# downloads the release src/script.rs names and fails if assets/datastar.js differs.
datastar version:
    scripts/datastar.sh {{version}}

# Open the release PR: `just release patch|minor|major`, or an exact version.
# Merging it publishes the crate and the GitHub release; tags are never pushed by hand.
release version *args:
    scripts/release.sh {{version}} {{args}}

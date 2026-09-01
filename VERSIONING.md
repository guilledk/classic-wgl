# Versioning

`classic-wgl` versions the **whole workspace as a single unit** using
[Semantic Versioning](https://semver.org/spec/v2.0.0.html), with a
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) file at
[`CHANGELOG.md`](CHANGELOG.md).

## Version scheme

- One version for the entire workspace: `[workspace.package.version]` in
  `Cargo.toml` (every crate uses `version.workspace = true`).
- Pre-1.0, the scheme is `0.MINOR.PATCH`:
  - **MINOR** — breaking changes (the 0.x semver rule: bump minor on any
    incompatible change).
  - **PATCH** — backwards-compatible fixes and new features.
- The current series carries a `-alpha.N` prerelease suffix. It is dropped
  when we promote to `0.1.0`; `xtask release` promotes (drops the suffix) on
  any bump, per cargo-release semantics. Use `xtask release --version X.Y.Z`
  to set a prerelease explicitly.

## What is *not* versioned here

- The ROM **format contract** (`format_version` in a ROM manifest) is owned by
  `classic-roms`, not by engine semver.
- Per-ROM and per-asset versions live in `classic-roms` and `classic-assets`
  respectively.

## Changelog

- `CHANGELOG.md` at the repo root, Keep-a-Changelog sections: `Added`,
  `Changed`, `Removed`, `Fixed`.
- Work in flight accumulates under `[Unreleased]`. Do **not** hand-edit a
  version number or bump `Cargo.toml` on a feature branch — that happens once,
  at release time.
- Each entry is one bullet per logical change, with the PR number in
  parentheses.

## When to release

Releases are cut **once per merge window**: after a stack of `wkt/*` branches
has been reviewed and merged to `master` (CI green), and `[Unreleased]` is
non-empty. Housekeeping-only windows (empty `[Unreleased]`) are skipped.

## How to release

Run the deterministic command — never hand-edit versions:

```bash
cargo xtask release <major|minor|patch>     # bump + freeze changelog + tag
cargo xtask release --version 0.2.0-alpha.0 # explicit version
```

`xtask release` performs, in order:

1. Bumps `[workspace.package.version]` in `Cargo.toml`.
2. Freezes `[Unreleased]` → `[<version>] - <date>` in `CHANGELOG.md`.
3. Verifies `Cargo.toml` version == top changelog version (`check-version`).
4. Prints the commit/tag commands to run (it does not mutate git).

Then commit, tag, and push (reviewing the frozen entry first):

```bash
git add Cargo.toml CHANGELOG.md
git commit -m "release v<version>"
git tag -a v<version> -m "release v<version>"
git push origin master --tags
```

Because ROMs are validated against a content-hash lock, re-pin them when the
release changes anything the golden baselines depend on:

```bash
cargo xtask lock-roms
CLASSIC_GOLDEN=update <golden invocation>   # see AGENTS.md
```

## Enforcing the invariant

- CI runs `cargo xtask check-version` on every PR/push, failing when
  `Cargo.toml` and `CHANGELOG.md` drift.
- CI also requires a `CHANGELOG.md` change when `crates/**` or `apps/**`
  change; label the PR `skip-changelog` to waive it for docs/housekeeping.

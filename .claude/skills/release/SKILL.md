---
name: classic-release
description: >
  Cut a classic-wgl release: freeze the changelog, bump the workspace version,
  and tag.  Use after a merge window lands on master and [Unreleased] is
  non-empty.  Covers `cargo xtask release` / `check-version`, the
  Keep-a-Changelog format, and the ROM-lock re-pin.  Trigger phrases: "release",
  "bump version", "cut a release", "freeze changelog", "tag vX.Y.Z",
  "version bump".
---

# Releasing classic-wgl

Releases are **one per merge window**, cut directly on `master` (no release
PR).  See [`VERSIONING.md`](../../VERSIONING.md) for the policy; this skill is
the runbook.

## When

- A stack of `wkt/*` branches has been reviewed and merged to `master`.
- CI is green (fmt, clippy, test, wasm-check, golden).
- `CHANGELOG.md`'s `[Unreleased]` section is **non-empty** — otherwise skip
  (housekeeping-only window).

## How

1. Confirm the tree is clean and on `master`:

   ```bash
   git status --short
   git branch --show-current
   ```

2. Choose the bump from the `[Unreleased]` content:
   - breaking change → `minor` (0.x rule),
   - fixes/features only → `patch`,
   - explicit prerelease → `--version X.Y.Z-alpha.N`.

3. Run the release (bumps `Cargo.toml`, freezes `CHANGELOG.md`, tags):

   ```bash
   cargo xtask release patch
   ```

4. Verify the frozen entry reads well, then push:

   ```bash
   git show --stat HEAD
   git push origin master --tags
   ```

5. Open a GitHub release with the new changelog section as the body.

6. If the release changed anything the golden baselines depend on, re-pin the
   ROM lock and re-baseline:

   ```bash
   cargo xtask lock-roms
   CLASSIC_HEADLESS=1 CLASSIC_FRAMES=60 CLASSIC_TEST=all CLASSIC_GOLDEN=update \
     cargo run -p classic-desktop
   ```

## Rules

- Never hand-edit `workspace.package.version` or a version heading on a feature
  branch — accumulate under `[Unreleased]` instead.
- Never bump the ROM `format_version` here; that is `classic-roms`' domain.
- One bullet per logical change, PR number in parentheses.
- Keep the changelog entries in Keep-a-Changelog order: `Added`, `Changed`,
  `Removed`, `Fixed`.

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

`classic-land` (its Phase 6) orchestrates the cross-repo release in pipeline
order — classic-assets → classic-roms → classic-wgl — and invokes this skill
for the per-repo mechanics.  Never cut a repo's release out of that order.

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

3. Run the release (bumps `Cargo.toml`, freezes `CHANGELOG.md`, and prints the
   commit/tag commands — it does not mutate git):

   ```bash
   cargo xtask release patch
   ```

4. Verify the frozen entry reads well, then commit + tag + push.  The bump also
   propagates to `Cargo.lock` (the workspace version appears in every crate
   entry), so include it in the release commit:

   ```bash
   git show --stat
   git add Cargo.toml CHANGELOG.md Cargo.lock
   git commit -m "release v<version>"
   git tag -a v<version> -m "release v<version>"
   git push origin master --tags
   ```

5. Open a GitHub release with the new changelog section as the body.

6. If a roms publish changed published ROM checksums since the last re-pin,
   re-pin the ROM lock and re-baseline (publish + re-pin move together):

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

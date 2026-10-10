# CLAUDE.md

Guidance for Claude Code sessions working in this repository, beyond what's discoverable
from the code itself.

## Workflow: everything goes through a pull request

`main` is protected by a GitHub ruleset with no bypass, including for the repo owner —
**direct pushes to `main` are rejected by GitHub itself**, not just discouraged by
convention. This applies to you too:

- Make changes on a branch, push the branch, open a PR with `gh pr create`.
- Don't merge it yourself unless explicitly asked to — the point of requiring a PR is
  that the user sees it first.
- This includes changes to this file, to workflows, to `release-plz.toml` — there are no
  exceptions carved out for tooling/config changes.

## Commit messages: Conventional Commits, because they drive releases

Version numbers are never hand-edited. [release-plz](https://release-plz.dev) computes
the next version from commit messages on `main` and opens a `chore: release vX.Y.Z` PR;
merging that PR is what cuts a release (see README.md's "Releasing" section for the full
mechanism). That means **the commit message you write is read by a machine, not just a
human** — get the prefix right:

| Prefix | Use for |
|---|---|
| `fix: …` | A bug fix |
| `feat: …` | A new feature |
| `feat!: …` / `fix!: …`, or a `BREAKING CHANGE:` footer | A breaking change |
| `chore:` / `docs:` / `style:` / `refactor:` / `test:` / `build:` / `ci:` | Anything that shouldn't trigger a release |

If you squash-merge a PR, only the final squash commit message matters for this — title
the PR accordingly, since GitHub pre-fills the squash message from the PR title.

While the version is below `1.0.0`, don't expect `feat` vs `fix` vs breaking to produce
different-looking version bumps — they all just increment the patch number by default
(standard pre-1.0 semver convention). Don't try to "fix" this by manually editing the
version or by reaching for `features_always_increment_minor`; it's intentional.

## Architecture: keep the OS out of `eye-saver-core`

`crates/core` has no OS dependencies (`#![deny(unsafe_code)]`, no file I/O, no clock
reads) and carries the ≥95% coverage requirement; `crates/platform/src/os/` and
`crates/app/` are the thin, untestable-without-a-desktop adapter layer and are excluded
from the coverage gate (see README.md's "Testing" section for the exact rationale — don't
inflate the number by moving logic there to dodge writing a test). When adding a feature,
default to putting the decision in `core` behind a trait in `core::ports`, with a real
implementation in `platform` and a recording fake in `core::testing`.

## Before calling anything done

Run `make check` (fmt, clippy `-D warnings`, tests, the 95% coverage gate) before
considering a change finished, and cross-check both OS's adapters compile even though you
likely only have one platform to hand:

```sh
cargo check -p eye-saver-platform --target x86_64-pc-windows-msvc
cargo check -p eye-saver-platform --target aarch64-pc-windows-msvc
cargo check -p eye-saver-core --target x86_64-apple-darwin   # if you're on Windows
```

## Don't

- Don't add a dependency that pulls in a webview, Electron, or a GUI plugin framework —
  the whole point of the `egui`/`glow`/`winit` stack is avoiding that.
- Don't skip `gl.clear()` in `crates/app/src/window.rs` if you touch the paint loop — its
  absence was a real, previously-shipped bug (stale frames bleeding through after a
  popup closed). The fix is the opaque `Frame` plus the explicit clear; keep both.
- Don't assume macOS has a tray icon to draw into — it doesn't any more (text-only status
  item, see `crates/platform/src/os/tray.rs`); only Windows draws the countdown as digits
  into the icon, because its notification area has no text label.

# ADR-0009: CachyOS-fidelity CI runner choice

**Date:** 2026-04-27
**Status:** Accepted
**Supersedes:** Plan §5.6 implementation (which assumed a published
CachyOS GHCR image and is no longer accurate)
**Superseded by:** —

## Context

Plan §5.6 specifies CI running inside `ghcr.io/cachyos/cachyos:latest`
"so paru, snapper, and nickel are present in the same versions
production users will see." On the very first CI run (M0 PR #1)
that container failed to pull — it isn't published publicly. M0 fell
back to plain `ubuntu-latest` plus pre-built tooling via
`taiki-e/install-action`, and M1 shipped on that.

M2 W1 lands apply-side adapter methods: `pearlite-pacman::install/remove`,
`pearlite-cargo::install/uninstall`, `pearlite-systemd::enable/disable/mask/restart`,
`pearlite-snapper::create/rollback/list`. Plan §6.6–§6.9 specify
**T3 live sub-suites** for each — small smoke tests that exercise a
real binary without needing a full VM. These don't run on a vanilla
Ubuntu runner because:

- `paru` isn't in apt; needs Arch's package format.
- `snapper` is in apt but needs btrfs subvolumes to do anything
  meaningful — same VM-tier requirement as on real CachyOS.
- `nickel-lang-cli` works fine on Ubuntu (already used in M1's CI).

Three options were evaluated:

1. **Publish `ghcr.io/steelbore/cachyos:latest`.** Highest fidelity
   to production. Costs: a Dockerfile, a build pipeline, weekly base
   refresh, image signing. Owned by Steelbore.
2. **Run T3 live on `archlinux:base` with packages installed at
   job-start.** Arch ≠ CachyOS — no v3/v4 repos, no
   `cachyos-settings` defaults — but `paru` and `snapper` install
   cleanly and the command surface is identical for the M2 work.
3. **Stay on `ubuntu-latest`; defer all live adapter tests to T4
   (self-hosted CachyOS qemu, runs nightly).** Cheapest. Worst
   feedback loop: a broken paru argv in a PR doesn't fail until the
   nightly run.

## Decision

**Adopt option 2 for M2; revisit at M3.** Add a fourth job to
`ci.yml` named `live-adapter` (T3-live) that uses
`archlinux:base` and installs `paru`, `snapper`, `nickel-lang`,
`btrfs-progs` at job-start. The live adapter sub-suites that Plan
§6.6–§6.9 enumerate run there on every PR.

Existing tiers stay as they are:

- T1 Lint — `ubuntu-latest` + `taiki-e/install-action` for cargo
  tooling.
- T2 Unit — same.
- T3 Adapter / audit — same. (Mocked integration plus cargo-deny.)
- **T3-live (new)** — `archlinux:base` with paru / snapper / nickel.
- T4 VM — self-hosted CachyOS qemu, nightly only.

Option 1 (publish our own image) is **deferred to M6** alongside the
sequoia-chameleon signing work. By then we'll have AUR PKGBUILD
release-pipeline expertise and the image-signing ergonomics will
overlap.

Option 3 (defer to T4) is rejected: nightly feedback is too slow for
the apply work, and the M2 W1 cadence is already tight.

## Consequences

- **One PR's worth of plumbing in M2 W1.** Add the new job to
  `ci.yml`. The adapter live sub-suites stay marked `#[ignore]` until
  the live job is wired; flipping them to `#[test]` is one line per
  test once the job exists.
- **Arch ≠ CachyOS leakage.** Tests that depend on
  `cachyos-settings` defaults or v3/v4 repos still need T4. The
  retrospective for M2 should call out any test that wants T3-live
  but actually needs T4.
- **No new infra to maintain.** `archlinux:base` is upstream-managed
  and rotates daily on Docker Hub.
- **Image-publish cost deferred.** When we revisit at M6 we'll have
  the option to upgrade to `ghcr.io/steelbore/cachyos` if T3-live
  has shown the gap matters in practice.

## Implementation

The `ci.yml` change lands with `pearlite-snapper` (the first M2 W1
crate) so the new job has a non-trivial test to exercise.

## References

- Plan §5.4 — CI tier table.
- Plan §5.6 — original ci.yml structure (now superseded).
- M0 PR #1 — first encounter with the missing CachyOS image.
- M1 retrospective `docs/retrospectives/M1.md` — surfaced this as
  M2 W1 action #1.

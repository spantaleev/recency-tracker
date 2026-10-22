# Agent notes

## Releases

Releases are CalVer Git tags `vYYYY.MM.DD.N` (UTC date; `N` counts from 0 within the day),
published as `ghcr.io/spantaleev/recency-tracker:YYYY.MM.DD.N` and `latest`. Nobody picks
versions or pushes tags by hand. Don't bump the version in `rust/Cargo.toml` either: it stays
`0.0.0`, and release builds embed the real version via `RECENCY_TRACKER_VERSION`.

`.github/workflows/release.yml` runs after every CI run on master, and every 30 minutes as a fallback
for releases that wait on Renovate PRs or the 90-minute cap.
[`rust/release/plan-release.py`](rust/release/plan-release.py) releases master's head commit when:

- something that ends up in the image changed since the last tag (`IMAGE_PATHS`; docs, tests
  and CI changes alone are never released)
- CI passed for that commit
- no open Renovate PR may still land (any that's not draft, conflicted, rejected or failing
  checks). This folds a burst of Renovate merges into one release.
- or, regardless of pending PRs, the commit is at least 90 minutes old

The run then rebuilds and validates both architectures with the version embedded, tags the
commit and publishes it. That takes about 20 minutes.

**Expected latency:** after a normal merge, CI takes about 10–20 minutes and the release about
another 20, so a release appears roughly 30–40 minutes later. A Renovate PR that is still
pending, or one ArtiDezi never merges, delays it until the next 30-minute check after the PR
settles or the 90-minute cap passes, plus the release time.

**Releasing immediately:** run the `Release` workflow manually from master (Actions → Release →
Run workflow, or `gh workflow run release.yml --repo spantaleev/recency-tracker`). This skips the
waiting but still requires image changes since the last release.

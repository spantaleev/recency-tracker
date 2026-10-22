#!/usr/bin/env python3
"""Decide whether the master commit this workflow run is for should be released now.

Releases are CalVer tags (vYYYY.MM.DD.N, UTC date, N counting from 0 per day) cut
without a human choosing a version. To fold a burst of Renovate merges into one
release, an unforced run waits while another Renovate PR may still land, but never
longer than QUIET_CAP after the last commit. Writes `release`, `version` and
`reason` to $GITHUB_OUTPUT.
"""
import datetime
import json
import os
import re
import subprocess
import sys

# Paths that end up in the published image (mirrors Dockerfile and .dockerignore).
IMAGE_PATHS = ["Dockerfile", ".dockerignore", "LICENSE.txt", "resources/static",
               "rust/Cargo.toml", "rust/Cargo.lock", "rust/src", "rust/container"]
QUIET_MIN = datetime.timedelta(minutes=15)
QUIET_CAP = datetime.timedelta(minutes=90)
CALVER = re.compile(r"v?([0-9]{4})\.([0-9]{2})\.([0-9]{2})\.([0-9]+)")
FAILED = {"FAILURE", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE", "ERROR"}


def parse(version):
    match = CALVER.fullmatch(version)
    return tuple(map(int, match.groups())) if match else None


def next_version(tags, today):
    released = [parse(tag) for tag in tags if tag.startswith("v") and parse(tag)]
    date = (today.year, today.month, today.day)
    if any(version[:3] > date for version in released):
        raise ValueError("a release tag is dated after today; refusing to go backward")
    counters = [version[3] for version in released if version[:3] == date]
    return f"{today:%Y.%m.%d}.{max(counters) + 1 if counters else 0}"


def may_land_soon(pr):
    """A Renovate PR that ArtiDezi could still merge (not draft, conflicted, rejected or red)."""
    if pr.get("isDraft") or pr.get("mergeable") == "CONFLICTING" or pr.get("reviewDecision") == "CHANGES_REQUESTED":
        return False
    return not any((check.get("conclusion") or check.get("state")) in FAILED
                   for check in pr.get("statusCheckRollup") or [])


def minutes(age):
    return f"{int(age.total_seconds() // 60)} min"


def decide(*, forced, changed, ci, age, landing):
    if not changed:
        return False, "nothing in the image changed since the last release"
    if forced:
        return True, "manually requested"
    if ci != "success":
        return False, f"CI for this commit is {ci}"
    if age >= QUIET_CAP:
        return True, f"master has been quiet for {minutes(age)} (cap reached)"
    if age < QUIET_MIN:
        return False, f"last commit is only {minutes(age)} old"
    if landing:
        return False, "waiting for Renovate PR(s) that may land soon: " + ", ".join(f"#{n}" for n in landing)
    return True, f"master has been quiet for {minutes(age)} and no Renovate PR is about to land"


def run(*command):
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout.strip()


def main():
    repository, sha, forced = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_SHA"], os.environ.get("FORCE") == "true"
    if os.environ["GITHUB_REF"] != "refs/heads/master":
        sys.exit("Releases are only cut from master")
    head = json.loads(run("gh", "api", f"repos/{repository}/git/ref/heads/master"))["object"]["sha"]
    if head != sha:
        release, reason = False, f"master has moved on to {head}; a later run decides for it"
    else:
        previous = run("git", "describe", "--tags", "--abbrev=0", "--match", "v[0-9]*", sha)
        changed = subprocess.run(["git", "diff", "--quiet", previous, sha, "--", *IMAGE_PATHS]).returncode != 0
        committed = datetime.datetime.fromtimestamp(int(run("git", "log", "-1", "--format=%ct", sha)), datetime.UTC)
        age = datetime.datetime.now(datetime.UTC) - committed
        runs = json.loads(run("gh", "api", f"repos/{repository}/actions/workflows/test.yml/runs?head_sha={sha}&event=push&per_page=1"))["workflow_runs"]
        ci = (runs[0]["conclusion"] or runs[0]["status"]) if runs else "missing"
        prs = json.loads(run("gh", "pr", "list", "--repo", repository, "--base", "master", "--author", "app/renovate",
                             "--json", "number,isDraft,mergeable,reviewDecision,statusCheckRollup"))
        landing = sorted(pr["number"] for pr in prs if may_land_soon(pr))
        release, reason = decide(forced=forced, changed=changed, ci=ci, age=age, landing=landing)
        reason += f" (previous release {previous})"
    version = next_version(run("git", "tag", "--list", "v*").splitlines(), datetime.datetime.now(datetime.UTC).date()) if release else ""
    print(f"{'Releasing ' + version if release else 'Not releasing'}: {reason}")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"release={str(release).lower()}\nversion={version}\nreason={reason}\n")
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        summary.write(f"{'Releasing `v' + version + '`' if release else 'Not releasing'}: {reason}\n")


if __name__ == "__main__":
    main()

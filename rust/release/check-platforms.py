#!/usr/bin/env python3
"""Require exactly the supported runtime platforms before publishing an index."""
import json
import sys


def validate(index):
    platforms = []
    for manifest in index.get("manifests", []):
        platform = manifest.get("platform", {})
        if platform.get("architecture") == "unknown":
            continue  # BuildKit provenance/SBOM attestations.
        platforms.append((platform.get("os"), platform.get("architecture"), platform.get("variant", "")))
    expected = {("linux", "amd64", ""), ("linux", "arm64", "")}
    # arm64/v8 and plain arm64 represent the same required ISA here.
    normalized = [(os_name, arch, "" if arch == "arm64" and variant == "v8" else variant)
                  for os_name, arch, variant in platforms]
    if len(normalized) != 2 or set(normalized) != expected:
        raise ValueError(f"expected exactly linux/amd64 and linux/arm64; got {platforms}")


if __name__ == "__main__":
    try:
        validate(json.load(sys.stdin))
    except (ValueError, KeyError, TypeError) as error:
        sys.exit(str(error))
    print("Image index contains linux/amd64 and linux/arm64")

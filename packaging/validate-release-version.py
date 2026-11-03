#!/usr/bin/env python3
"""Keep the Cargo and Snap versions aligned and optionally validate a tag."""

import argparse
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="release tag to compare, for example v0.1.0")
    args = parser.parse_args()

    with (ROOT / "Cargo.toml").open("rb") as stream:
        cargo_version = tomllib.load(stream)["package"]["version"]
    snapcraft = (ROOT / "packaging/snap/snapcraft.yaml").read_text()
    match = re.search(r"(?m)^version:\s*['\"]?([^'\"\s]+)['\"]?\s*$", snapcraft)
    if match is None:
        raise SystemExit("Snapcraft version field was not found")
    if match.group(1) != cargo_version:
        raise SystemExit(
            f"Cargo version {cargo_version} does not match Snap version {match.group(1)}"
        )
    if args.tag and args.tag != f"v{cargo_version}":
        raise SystemExit(f"tag {args.tag} does not match package version v{cargo_version}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Ensure a release tag matches the package version in Cargo.toml."""

import argparse
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True, help="release tag, for example v1.0.0")
    args = parser.parse_args()

    with (ROOT / "Cargo.toml").open("rb") as stream:
        package_version = tomllib.load(stream)["package"]["version"]
    expected_tag = f"v{package_version}"
    if args.tag != expected_tag:
        raise SystemExit(f"tag {args.tag} does not match package version {expected_tag}")


if __name__ == "__main__":
    main()

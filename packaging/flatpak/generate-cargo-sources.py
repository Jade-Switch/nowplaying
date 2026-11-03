#!/usr/bin/env python3
"""Generate flatpak-builder source entries from this project's Cargo.lock."""

import argparse
import json
import tomllib
from pathlib import Path


CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"


def generate(lockfile: Path) -> list[dict[str, str]]:
    with lockfile.open("rb") as stream:
        lock = tomllib.load(stream)

    sources: list[dict[str, str]] = []
    for package in lock["package"]:
        source = package.get("source")
        if source is None:
            continue
        if source != CRATES_IO:
            raise ValueError(
                f"unsupported Cargo source for {package['name']}: {source}"
            )

        name = package["name"]
        version = package["version"]
        checksum = package.get("checksum")
        if not checksum:
            raise ValueError(f"missing registry checksum for {name} {version}")

        destination = f"cargo/vendor/{name}-{version}"
        sources.extend(
            [
                {
                    "type": "archive",
                    "archive-type": "tar-gzip",
                    "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                    "sha256": checksum,
                    "dest": destination,
                },
                {
                    "type": "inline",
                    "contents": json.dumps({"package": checksum, "files": {}}),
                    "dest": destination,
                    "dest-filename": ".cargo-checksum.json",
                },
            ]
        )

    sources.append(
        {
            "type": "inline",
            "contents": (
                '[source.crates-io]\n'
                'replace-with = "vendored-sources"\n\n'
                '[source.vendored-sources]\n'
                'directory = "cargo/vendor"\n'
            ),
            "dest": "cargo",
            "dest-filename": "config",
        }
    )
    return sources


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--lockfile", type=Path, default=Path("Cargo.lock"),
        help="Cargo lockfile to use (default: Cargo.lock)",
    )
    parser.add_argument(
        "--output", type=Path,
        default=Path(__file__).with_name("cargo-sources.json"),
        help="generated Flatpak source list",
    )
    args = parser.parse_args()
    args.output.write_text(json.dumps(generate(args.lockfile), indent=2) + "\n")


if __name__ == "__main__":
    main()

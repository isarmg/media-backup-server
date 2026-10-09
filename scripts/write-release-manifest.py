#!/usr/bin/env python3
"""Create the current strict xszs release manifest."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tomllib
from typing import NoReturn


IDENTITY_KEYS = {
    "product",
    "version",
    "source_revision",
    "target",
    "api_version",
    "storage_encoding",
    "server_schema_revision",
    "server_schema_sha256",
    "web_assets_sha256",
    "release_contract_sha256",
}

EXPECTED_FILES = {
    "LICENSE": 0o644,
    "bin/xszs": 0o755,
    "config/xszs.env.example": 0o644,
    "docs/feature-inventory-and-tradeoffs.md": 0o644,
    "README.md": 0o644,
    "scripts/run-server-wsl.sh": 0o755,
    "scripts/setup-wsl.sh": 0o755,
    "scripts/start-server-wsl.sh": 0o755,
    "scripts/verify-server-wsl.sh": 0o755,
    "share/web-assets.json": 0o644,
    "systemd/xszs.service": 0o644,
}


def fail(message: str) -> NoReturn:
    raise SystemExit(f"manifest error: {message}")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(64 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def read_software_version() -> str:
    manifest = Path(__file__).resolve().parents[1] / "Cargo.toml"
    with manifest.open("rb") as source:
        version = tomllib.load(source)["workspace"]["package"]["version"]
    if not isinstance(version, str) or len(version.split(".")) != 3 or any(
        not item.isascii() or not item.isdecimal() for item in version.split(".")
    ):
        fail("Cargo workspace software version is invalid")
    return version


def validate_identity(identity: object, revision: str, version: str) -> None:
    if not isinstance(identity, dict) or set(identity) != IDENTITY_KEYS:
        fail("binary release identity has an unknown or missing field")
    fixed_identity = {
        "product": "xszs",
        "version": version,
        "source_revision": revision,
        "target": "x86_64-unknown-linux-gnu",
        "api_version": "v1",
        "storage_encoding": "plain-v1",
        "server_schema_revision": 1,
    }
    for field, expected in fixed_identity.items():
        if identity.get(field) != expected:
            fail(f"binary release identity mismatch for {field}")
    for field in (
        "server_schema_sha256",
        "web_assets_sha256",
        "release_contract_sha256",
    ):
        value = identity.get(field)
        if not isinstance(value, str) or len(value) != 64 or any(
            character not in "0123456789abcdef" for character in value
        ):
            fail(f"binary release identity has an invalid {field}")


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: write-release-manifest.py RELEASE_ROOT SOURCE_REVISION")
    root = Path(sys.argv[1])
    revision = sys.argv[2]
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        fail("release root must be an absolute real directory")
    if len(revision) != 40 or any(character not in "0123456789abcdef" for character in revision):
        fail("source revision must be 40 lowercase hexadecimal characters")

    binary = root / "bin/xszs"
    completed = subprocess.run(
        [str(binary), "release-identity"],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if completed.stderr:
        fail("release identity wrote unexpected stderr output")
    try:
        identity = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        fail(f"binary release identity is not JSON: {error}")
    validate_identity(identity, revision, read_software_version())

    files = []
    for relative, expected_mode in sorted(EXPECTED_FILES.items()):
        path = root / relative
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or path.is_symlink():
            fail(f"payload is not a regular file: {relative}")
        if metadata.st_nlink != 1:
            fail(f"payload has a hard-link alias: {relative}")
        if stat.S_IMODE(metadata.st_mode) != expected_mode:
            fail(f"payload has the wrong mode: {relative}")
        files.append(
            {
                "path": relative,
                "mode": expected_mode,
                "size": metadata.st_size,
                "sha256": sha256_file(path),
            }
        )

    manifest = {
        "manifest_version": 1,
        "identity": identity,
        "files": files,
    }
    destination = root / "release-manifest.json"
    descriptor = os.open(
        destination,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        0o644,
    )
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(manifest, output, ensure_ascii=False, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        try:
            destination.unlink()
        except FileNotFoundError:
            pass
        raise
    destination.chmod(0o644)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Create the current strict Media Backup Server release manifest."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
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
    "bin/media-backup-server": 0o755,
    "config/media-backup.env.example": 0o644,
    "docs/feature-inventory-and-tradeoffs.md": 0o644,
    "README.md": 0o644,
    "scripts/run-server-wsl.sh": 0o755,
    "scripts/setup-wsl.sh": 0o755,
    "scripts/start-server-wsl.sh": 0o755,
    "scripts/verify-server-wsl.sh": 0o755,
    "share/web/index.html": 0o644,
    "share/web/assets/admin.js": 0o644,
    "share/web/assets/admin.css": 0o644,
    "share/web/assets/CJK-LICENSE.txt": 0o644,
    "share/web/assets/MapleMonoBootstrap-Regular.woff2": 0o644,
    "share/web/assets/MapleMonoBootstrap-Bold.woff2": 0o644,
    "share/web/assets/MapleMonoNormalNL-Regular.woff2": 0o644,
    "share/web/assets/MapleMonoNormalNL-Bold.woff2": 0o644,
    "share/web/assets/MapleMono-OFL.txt": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-4D4-50CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-50CF-52CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-52CF-54CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-54CF-56CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-56CF-58CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-58CF-5ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5ACF-5CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5CCF-5ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-5ECF-60CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-60CF-62CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-62CF-64CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-64CF-66CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-66CF-68CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-68CF-6ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6ACF-6CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6CCF-6ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-6ECF-70CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-70CF-72CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-72CF-74CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-74CF-76CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-76CF-78CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-78CF-7ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7ACF-7CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7CCF-7ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-7ECF-80CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-80CF-82CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-82CF-84CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-84CF-86CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-86CF-88CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-88CF-8ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8ACF-8CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8CCF-8ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-8ECF-90CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-90CF-92CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-92CF-94CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-94CF-96CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-96CF-98CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-98CF-9ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9ACF-9CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9CCF-9E67.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Bold-9E68-FFEE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-4D4-50CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-50CF-52CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-52CF-54CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-54CF-56CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-56CF-58CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-58CF-5ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5ACF-5CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5CCF-5ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-5ECF-60CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-60CF-62CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-62CF-64CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-64CF-66CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-66CF-68CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-68CF-6ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6ACF-6CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6CCF-6ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-6ECF-70CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-70CF-72CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-72CF-74CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-74CF-76CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-76CF-78CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-78CF-7ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7ACF-7CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7CCF-7ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-7ECF-80CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-80CF-82CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-82CF-84CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-84CF-86CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-86CF-88CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-88CF-8ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8ACF-8CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8CCF-8ECE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-8ECF-90CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-90CF-92CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-92CF-94CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-94CF-96CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-96CF-98CE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-98CF-9ACE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9ACF-9CCE.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9CCF-9E67.woff2": 0o644,
    "share/web/assets/MapleMonoNL-CN-Regular-9E68-FFEE.woff2": 0o644,
    "systemd/media-backup.service": 0o644,
}


def fail(message: str) -> NoReturn:
    raise SystemExit(f"manifest error: {message}")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(64 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: write-release-manifest.py RELEASE_ROOT SOURCE_REVISION")
    root = Path(sys.argv[1])
    revision = sys.argv[2]
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        fail("release root must be an absolute real directory")
    if len(revision) != 40 or any(character not in "0123456789abcdef" for character in revision):
        fail("source revision must be 40 lowercase hexadecimal characters")

    binary = root / "bin/media-backup-server"
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
    if not isinstance(identity, dict) or set(identity) != IDENTITY_KEYS:
        fail("binary release identity has an unknown or missing field")
    fixed_identity = {
        "product": "media-backup-server",
        "version": "0.3.21",
        "source_revision": revision,
        "target": "x86_64-unknown-linux-gnu",
        "api_version": "v2",
        "storage_encoding": "plain-v1",
        "server_schema_revision": 5,
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

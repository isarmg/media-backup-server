"""The release manifest must bind the actual software and source identity."""

import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "release_manifest", Path(__file__).with_name("write-release-manifest.py")
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ReleaseManifestIdentityTests(unittest.TestCase):
    def identity(self):
        return {
            "product": "xszs",
            "version": MODULE.read_software_version(),
            "source_revision": "a" * 40,
            "target": "x86_64-unknown-linux-gnu",
            "api_version": "v1",
            "storage_encoding": "plain-v1",
            "server_schema_revision": 1,
            "server_schema_sha256": "b" * 64,
            "web_assets_sha256": "c" * 64,
            "release_contract_sha256": "d" * 64,
        }

    def test_actual_cargo_version_with_complete_release_identity(self):
        identity = self.identity()
        MODULE.validate_identity(identity, "a" * 40, MODULE.read_software_version())

    def test_wrong_version_source_target_or_missing_digest_is_rejected(self):
        for field, invalid in [
            ("version", "0.4.0"),
            ("source_revision", "f" * 40),
            ("target", "aarch64-unknown-linux-gnu"),
            ("server_schema_sha256", "invalid"),
        ]:
            with self.subTest(field=field):
                identity = self.identity()
                identity[field] = invalid
                with self.assertRaises(SystemExit):
                    MODULE.validate_identity(
                        identity, "a" * 40, MODULE.read_software_version()
                    )
        identity = self.identity()
        del identity["web_assets_sha256"]
        with self.assertRaises(SystemExit):
            MODULE.validate_identity(identity, "a" * 40, MODULE.read_software_version())


if __name__ == "__main__":
    unittest.main()

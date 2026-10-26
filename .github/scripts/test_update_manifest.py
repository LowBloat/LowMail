import tempfile
import unittest
from pathlib import Path

from update_manifest import build_manifest


class ManifestTest(unittest.TestCase):
    def test_signed_assets_become_platforms_and_linux_stays_appimage_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets = Path(tmp)
            for name in [
                "LowMail-1.2.3-windows-x64-setup.exe",
                "LowMail-1.2.3-windows-x64.msi",
                "LowMail-1.2.3-macos-universal.app.tar.gz",
                "LowMail-1.2.3-linux-x64.AppImage",
                "LowMail-1.2.3-linux-x64.deb",
            ]:
                (assets / name).write_bytes(b"x")
                if not name.endswith(".deb"):
                    (assets / f"{name}.sig").write_text(f"sig-{name}\n")
            doc = build_manifest("1.2.3", "LowBloat/LowMail", assets, "notes")
        self.assertEqual(
            sorted(doc["platforms"]),
            ["darwin-aarch64", "darwin-x86_64", "linux-x86_64-appimage",
             "windows-x86_64", "windows-x86_64-msi", "windows-x86_64-nsis"],
        )
        self.assertNotIn("linux-x86_64", doc["platforms"])
        self.assertEqual(doc["platforms"]["windows-x86_64-msi"]["signature"], "sig-LowMail-1.2.3-windows-x64.msi")
        self.assertTrue(doc["platforms"]["windows-x86_64"]["url"].endswith("/v1.2.3/LowMail-1.2.3-windows-x64-setup.exe"))

    def test_unsigned_assets_are_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            assets = Path(tmp)
            (assets / "LowMail-1.2.3-windows-x64-setup.exe").write_bytes(b"x")
            doc = build_manifest("1.2.3", "LowBloat/LowMail", assets, "notes")
        self.assertEqual(doc["platforms"], {})


if __name__ == "__main__":
    unittest.main()

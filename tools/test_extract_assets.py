#!/usr/bin/env python3
import io
import json
import os
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest import mock

from extract_assets import extract_archive, run_extraction


class ExtractAssetsTests(unittest.TestCase):
    def test_extracts_from_app_bundle_deduplicates_and_writes_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            app_dir = tmp_path / "HandShaker.app" / "Contents" / "Resources"
            (app_dir / "nested").mkdir(parents=True)
            png_bytes = b"\x89PNG\r\n\x1a\n" + b"A" * 128
            wav_bytes = b"RIFF" + b"\x00" * 64 + b"WAVE"
            (app_dir / "HandShakerIcon.png").write_bytes(png_bytes)
            (app_dir / "nested" / "duplicate.png").write_bytes(png_bytes)
            (app_dir / "connect.wav").write_bytes(wav_bytes)
            (app_dir / "tiny.png").write_bytes(b"tiny")

            out_dir = tmp_path / "out"
            with mock.patch.dict(os.environ, {"XDG_DATA_HOME": str(tmp_path / "xdg")}):
                manifest = run_extraction(
                    source=tmp_path / "HandShaker.app",
                    output_dir=out_dir,
                    dry_run=False,
                    install_icon=True,
                )

            self.assertEqual(manifest["counts"], {"images": 1, "sounds": 1, "icons": 0, "total": 2})
            self.assertTrue((out_dir / "images" / "HandShakerIcon.png").is_file())
            self.assertTrue((out_dir / "sounds" / "connect.wav").is_file())
            self.assertTrue((out_dir / "manifest.json").is_file())
            installed_icon = Path(manifest["icon_installed_to"])
            self.assertTrue(installed_icon.is_file())
            self.assertEqual(installed_icon.read_bytes(), png_bytes)

            loaded = json.loads((out_dir / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(loaded["counts"]["total"], 2)

    def test_dry_run_does_not_create_output(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            src = tmp_path / "unpacked"
            src.mkdir()
            (src / "sound.ogg").write_bytes(b"OggS" + b"\x01" * 64)
            out_dir = tmp_path / "never_created"

            manifest = run_extraction(source=src, output_dir=out_dir, dry_run=True)
            self.assertEqual(manifest["counts"]["sounds"], 1)
            self.assertFalse(out_dir.exists())

    def test_zip_extraction_blocks_path_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            zpath = tmp_path / "evil.zip"
            buf = io.BytesIO()
            with zipfile.ZipFile(buf, "w") as zf:
                zf.writestr("../../outside.png", b"X" * 100)
            zpath.write_bytes(buf.getvalue())

            with self.assertRaises(ValueError):
                extract_archive(zpath, tmp_path / "unpack")


if __name__ == "__main__":
    unittest.main()

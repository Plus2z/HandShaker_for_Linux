#!/usr/bin/env python3
"""Runtime UI asset & sound extractor for user-supplied official HandShaker packages.

Copyrighted Smartisan HandShaker assets are never bundled with HandShaker for Linux.
Users who own a copy of the official macOS (.dmg/.app) or Windows installer/install
directory can extract icons, textures, and notification sounds locally into:

    $XDG_DATA_HOME/handshaker-linux/assets/
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable

IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".webp", ".svg"}
SOUND_EXTS = {".wav", ".mp3", ".ogg", ".m4a", ".aiff", ".aif"}
ICON_EXTS = {".icns", ".ico"}

MAX_SINGLE_ASSET_BYTES = 32 * 1024 * 1024
MIN_ASSET_BYTES = 32


@dataclass(frozen=True)
class ExtractedAsset:
    category: str
    filename: str
    relative_source: str
    size_bytes: int
    sha256: str


def default_output_dir() -> Path:
    xdg = os.environ.get("XDG_DATA_HOME", "").strip()
    base = Path(xdg) if xdg else Path.home() / ".local" / "share"
    return base / "handshaker-linux" / "assets"


def xdg_hicolor_icon_path() -> Path:
    xdg = os.environ.get("XDG_DATA_HOME", "").strip()
    base = Path(xdg) if xdg else Path.home() / ".local" / "share"
    return base / "icons" / "hicolor" / "128x128" / "apps" / "handshaker-linux.png"


def classify_extension(path: Path) -> str | None:
    ext = path.suffix.lower()
    if ext in IMAGE_EXTS:
        return "images"
    if ext in SOUND_EXTS:
        return "sounds"
    if ext in ICON_EXTS:
        return "icons"
    return None


def sanitize_filename(name: str) -> str:
    clean = re.sub(r"[^A-Za-z0-9._-]+", "_", name).strip("._")
    return clean or "asset.bin"


def extract_archive(archive_path: Path, dest_dir: Path) -> None:
    suffix = archive_path.suffix.lower()
    if suffix == ".zip":
        dest_root = dest_dir.resolve()
        with zipfile.ZipFile(archive_path, "r") as zf:
            for member in zf.infolist():
                if member.is_dir():
                    continue
                target = (dest_root / member.filename).resolve()
                if not target.is_relative_to(dest_root):
                    raise ValueError(f"Unsafe zip member path blocked: {member.filename!r}")
                target.parent.mkdir(parents=True, exist_ok=True)
                with zf.open(member, "r") as src, target.open("wb") as dst:
                    shutil.copyfileobj(src, dst)
        return

    seven_z = shutil.which("7z") or shutil.which("7zz")
    if seven_z:
        proc = subprocess.run(
            [seven_z, "x", "-y", f"-o{dest_dir}", str(archive_path)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if proc.returncode == 0:
            return
        err = proc.stderr.strip() or proc.stdout.strip()
        raise RuntimeError(f"7z failed extracting {archive_path.name}: {err}")

    bsdtar = shutil.which("bsdtar")
    if bsdtar and suffix in {".tar", ".gz", ".tgz", ".xz", ".bz2", ".dmg"}:
        proc = subprocess.run(
            [bsdtar, "-xf", str(archive_path), "-C", str(dest_dir)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if proc.returncode == 0:
            return
        raise RuntimeError(f"bsdtar failed extracting {archive_path.name}: {proc.stderr.strip()}")

    raise RuntimeError(
        f"Cannot unpack {archive_path.name}: install '7z' (p7zip / 7zip) or 'bsdtar', "
        "or pass an already unpacked directory / .app bundle."
    )


def iter_safe_files(root: Path) -> Iterable[Path]:
    root_resolved = root.resolve()
    for dirpath, dirnames, filenames in os.walk(root_resolved, followlinks=False):
        current = Path(dirpath)
        dirnames[:] = sorted(d for d in dirnames if not (current / d).is_symlink())
        for fname in sorted(filenames):
            candidate = current / fname
            if candidate.is_symlink():
                continue
            resolved = candidate.resolve()
            if not resolved.is_relative_to(root_resolved):
                continue
            yield resolved


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def collect_assets(
    source_root: Path,
    output_dir: Path,
    dry_run: bool = False,
    install_icon: bool = False,
) -> dict:
    root_resolved = source_root.resolve()
    seen_hashes: set[str] = set()
    used_names: dict[str, set[str]] = {"images": set(), "sounds": set(), "icons": set()}
    extracted: list[ExtractedAsset] = []

    for candidate in iter_safe_files(root_resolved):
        category = classify_extension(candidate)
        if not category:
            continue
        size = candidate.stat().st_size
        if size < MIN_ASSET_BYTES or size > MAX_SINGLE_ASSET_BYTES:
            continue

        digest = sha256_file(candidate)
        if digest in seen_hashes:
            continue
        seen_hashes.add(digest)

        rel = candidate.relative_to(root_resolved).as_posix()
        base_name = sanitize_filename(candidate.name)
        stem = Path(base_name).stem
        ext = Path(base_name).suffix.lower()
        target_name = base_name
        if target_name in used_names[category]:
            target_name = f"{stem}_{digest[:8]}{ext}"
        used_names[category].add(target_name)

        if not dry_run:
            cat_dir = output_dir / category
            cat_dir.mkdir(parents=True, exist_ok=True)
            shutil.copy2(candidate, cat_dir / target_name)

        extracted.append(
            ExtractedAsset(
                category=category,
                filename=target_name,
                relative_source=rel,
                size_bytes=size,
                sha256=digest,
            )
        )

    icon_installed_to: str | None = None
    if install_icon and not dry_run:
        png_candidates = [
            a for a in extracted
            if a.category == "images"
            and a.filename.lower().endswith(".png")
            and any(tok in a.relative_source.lower() for tok in ("handshaker", "appicon", "logo", "icon"))
        ]
        if png_candidates:
            best = max(png_candidates, key=lambda a: a.size_bytes)
            src_png = output_dir / "images" / best.filename
            dst_icon = xdg_hicolor_icon_path()
            dst_icon.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src_png, dst_icon)
            icon_installed_to = str(dst_icon)

    manifest = {
        "schema_version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "output_dir": str(output_dir),
        "dry_run": dry_run,
        "counts": {
            "images": sum(1 for a in extracted if a.category == "images"),
            "sounds": sum(1 for a in extracted if a.category == "sounds"),
            "icons": sum(1 for a in extracted if a.category == "icons"),
            "total": len(extracted),
        },
        "icon_installed_to": icon_installed_to,
        "assets": [asdict(a) for a in extracted],
    }

    if not dry_run:
        output_dir.mkdir(parents=True, exist_ok=True)
        (output_dir / "manifest.json").write_text(
            json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )

    return manifest


def run_extraction(
    source: Path,
    output_dir: Path,
    dry_run: bool = False,
    install_icon: bool = False,
) -> dict:
    if not source.exists():
        raise FileNotFoundError(f"Source path does not exist: {source}")

    if source.is_dir():
        app_resources = source / "Contents" / "Resources"
        scan_root = app_resources if app_resources.is_dir() else source
        return collect_assets(scan_root, output_dir, dry_run=dry_run, install_icon=install_icon)

    with tempfile.TemporaryDirectory(prefix="handshaker-assets-") as tmp:
        tmp_path = Path(tmp)
        extract_archive(source, tmp_path)
        return collect_assets(tmp_path, output_dir, dry_run=dry_run, install_icon=install_icon)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract icons, images, and sounds from an official HandShaker installer or app directory.",
    )
    parser.add_argument(
        "source",
        type=Path,
        help="Path to official HandShaker.dmg, HandShaker.app, Windows setup .exe, .zip, or unpacked folder.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=default_output_dir(),
        help="Destination directory (default: $XDG_DATA_HOME/handshaker-linux/assets).",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Scan and report matching assets without writing files.",
    )
    parser.add_argument(
        "--install-icon",
        action="store_true",
        help="Copy the best matching PNG app icon into ~/.local/share/icons/hicolor/128x128/apps/.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Print full JSON manifest to stdout.",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        manifest = run_extraction(
            source=args.source,
            output_dir=args.output,
            dry_run=args.dry_run,
            install_icon=args.install_icon,
        )
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps(manifest, indent=2, ensure_ascii=False))
    else:
        c = manifest["counts"]
        mode = "Dry-run found" if args.dry_run else "Extracted"
        print(
            f"{mode} {c['total']} assets "
            f"(images={c['images']}, sounds={c['sounds']}, icons={c['icons']}) "
            f"-> {manifest['output_dir']}"
        )
        if manifest["icon_installed_to"]:
            print(f"Installed launcher icon -> {manifest['icon_installed_to']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

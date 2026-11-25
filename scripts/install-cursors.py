#!/usr/bin/env python3
"""Explicit, user-local installation of upstream cursor assets; no scripts executed."""
import argparse
import gzip
import lzma
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import sys
import tarfile
import tempfile
import urllib.request

BIBATA = {
    "Modern-Amber-Right": 1916728, "Modern-Amber": 1902476,
    "Modern-Classic-Right": 1778428, "Modern-Classic": 1767748,
    "Modern-Ice-Right": 1709720, "Modern-Ice": 1701468,
    "Original-Amber-Right": 1887908, "Original-Amber": 2016504,
    "Original-Classic-Right": 1763224, "Original-Classic": 1871780,
    "Original-Ice-Right": 1689800, "Original-Ice": 1794192,
}
GOOGLE = {"Black": 4785772, "Blue": 4854347, "Red": 4329016, "White": 4623283}
ASSETS = {
    **{"Bibata-" + name: (
        "https://github.com/ful1e5/Bibata_Cursor/releases/download/v2.0.7/Bibata-"
        + name + ".tar.xz", size, "xz") for name, size in BIBATA.items()},
    **{"GoogleDot-" + name: (
        "https://github.com/ful1e5/Google_Cursor/releases/download/v2.0.0/GoogleDot-"
        + name + ".tar.gz", size, "gz") for name, size in GOOGLE.items()},
}
MAX_FILE = 8 * 1024 * 1024
MAX_EXPANDED = 192 * 1024 * 1024
MAX_ENTRIES = 4096
NAME = re.compile(r"[A-Za-z0-9_.+-]{1,160}\Z")


class BoundedReader:
    def __init__(self, source):
        self.source = source
        self.remaining = MAX_EXPANDED

    def read(self, count):
        if count < 0 or count > self.remaining:
            raise ValueError("archive exceeds expanded-size limit")
        data = self.source.read(count)
        self.remaining -= len(data)
        return data


def safe_name(value):
    return value not in (".", "..") and NAME.fullmatch(value) is not None


def extract_theme(archive, compression, theme, stage):
    """Accept only theme metadata and cursor images/aliases; never extract arbitrary paths."""
    cursors = stage / "cursors"
    cursors.mkdir()
    files, aliases, metadata = set(), {}, set()
    total = 0
    opener = lzma.open if compression == "xz" else gzip.open
    with opener(archive, "rb") as decoded, tarfile.open(
        fileobj=BoundedReader(decoded), mode="r|"
    ) as tar:
        for index, member in enumerate(tar):
            if index >= MAX_ENTRIES:
                raise ValueError("too many archive entries")
            raw = member.name
            parts = PurePosixPath(raw).parts
            if raw.startswith("/") or ".." in parts or "\\" in raw:
                raise ValueError("unsafe archive path")
            if parts and parts[0] == theme:
                parts = parts[1:]
            if not parts:
                if not member.isdir():
                    raise ValueError("unexpected archive root")
                continue
            if len(parts) == 1 and parts[0] == "cursors" and member.isdir():
                continue
            if len(parts) == 1 and parts[0] in ("index.theme", "cursor.theme"):
                if not member.isfile() or member.size > 65536 or parts[0] in metadata:
                    raise ValueError("invalid theme metadata")
                metadata.add(parts[0])
                with (stage / parts[0]).open("xb") as out:
                    shutil.copyfileobj(tar.extractfile(member), out)
                continue
            if len(parts) == 1 and parts[0] in ("LICENSE", "LICENSE.txt", "COPYING", "README.md"):
                # Preserve attribution files as inert regular data, when present.
                if not member.isfile() or member.size > 65536:
                    raise ValueError("invalid attribution file")
                with (stage / parts[0]).open("xb") as out:
                    shutil.copyfileobj(tar.extractfile(member), out)
                continue
            if len(parts) != 2 or parts[0] != "cursors" or not safe_name(parts[1]):
                raise ValueError(f"unexpected archive member: {raw!r}")
            name = parts[1]
            if name in files or name in aliases:
                raise ValueError("duplicate cursor entry")
            if member.issym():
                if not safe_name(member.linkname):
                    raise ValueError("unsafe cursor alias")
                aliases[name] = member.linkname
            elif member.islnk():
                target = PurePosixPath(member.linkname).parts
                if target[:1] == (theme,):
                    target = target[1:]
                if len(target) != 2 or target[0] != "cursors" or not safe_name(target[1]):
                    raise ValueError("unsafe cursor hardlink")
                aliases[name] = target[1]
            elif member.isfile():
                total += member.size
                if not 4 <= member.size <= MAX_FILE or total > MAX_EXPANDED:
                    raise ValueError("cursor image size exceeds limit")
                source = tar.extractfile(member)
                if source.read(4) != b"Xcur":
                    raise ValueError("non-XCursor image in cursor directory")
                with (cursors / name).open("xb") as out:
                    out.write(b"Xcur")
                    shutil.copyfileobj(source, out)
                files.add(name)
            else:
                raise ValueError("unsupported archive entry")
    # Flatten aliases to a verified regular image within the same directory.
    for name, target in aliases.items():
        visited = {name}
        while target in aliases:
            if target in visited:
                raise ValueError("cyclic cursor aliases")
            visited.add(target)
            target = aliases[target]
        if target not in files:
            raise ValueError("dangling cursor alias")
        (cursors / name).symlink_to(target)
    if not files or not {"default", "left_ptr"}.intersection(files | aliases.keys()):
        raise ValueError("theme lacks a usable default pointer")
    if "index.theme" not in metadata:
        (stage / "index.theme").write_text(f"[Icon Theme]\nName={theme}\n", encoding="utf-8")


def download(url, expected, target):
    with urllib.request.urlopen(url, timeout=30) as response, target.open("xb") as out:
        if not response.geturl().startswith("https://"):
            raise ValueError("non-HTTPS download redirect")
        remaining = expected
        while remaining:
            data = response.read(min(65536, remaining))
            if not data:
                raise ValueError("truncated release asset")
            out.write(data)
            remaining -= len(data)
        if response.read(1):
            raise ValueError("asset exceeds pinned size")


def install(theme, root):
    url, expected, compression = ASSETS[theme]
    destination = root / theme
    if os.path.lexists(destination):
        raise FileExistsError(f"Refusing to overwrite {destination}")
    with tempfile.TemporaryDirectory(prefix=".atelier-cursor-", dir=root) as tmp:
        work = Path(tmp)
        archive = work / "asset"
        stage = work / "theme"
        stage.mkdir()
        download(url, expected, archive)
        extract_theme(archive, compression, theme, stage)
        # mkdir reserves a new destination and never replaces an existing one.
        destination.mkdir(mode=0o755)
        try:
            for child in stage.iterdir():
                child.rename(destination / child.name)
        except BaseException:
            shutil.rmtree(destination)
            raise
    print(f"Installed {theme}: {destination}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("themes", nargs="*", choices=None)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--install", action="store_true", help="Approve the listed downloads and installation")
    args = parser.parse_args()
    if args.list:
        for theme in sorted(ASSETS):
            print(theme)
        return
    if not args.themes or any(theme not in ASSETS for theme in args.themes):
        parser.error("choose theme IDs from --list")
    if len(set(args.themes)) != len(args.themes):
        parser.error("duplicate theme selection")
    if os.geteuid() == 0:
        parser.error("run as your desktop user, not root")
    root = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "icons"
    if not root.is_absolute():
        parser.error("XDG_DATA_HOME must be absolute")
    print("Upstream GPL-3.0 cursor assets. Pinned release and byte size; no published checksum.")
    for theme in args.themes:
        print(f"{theme}: {ASSETS[theme][0]} -> {root / theme}")
    if not args.install:
        print("Preview only. Pass --install to approve; no sudo or AUR helper.")
        return
    root.mkdir(parents=True, exist_ok=True)
    if root.stat().st_uid != os.geteuid() or root.stat().st_mode & 0o022:
        parser.error("icons directory must be user-owned and not writable by other users")
    for theme in args.themes:
        if os.path.lexists(root / theme):
            parser.error(f"existing target: {root / theme}")
    for theme in args.themes:
        install(theme, root)
    print("Choose the installed theme in Atelier Settings and restart the native session.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"Cursor installation stopped: {error}", file=sys.stderr)
        sys.exit(1)

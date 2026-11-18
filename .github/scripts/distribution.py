#!/usr/bin/env python3
"""Offline native distribution PREPARATION, not the immutable 0.1.5 release path.

Python 3.11+, stdlib. `build --target TRIPLE` invokes locked Cargo; `package
--target TRIPLE --version X.Y.Z --output DIR` packages an existing target release
binary. No publishing, update feeds, credentials, Developer ID or notarization.
macOS retains mandatory ad-hoc integrity signing (codesign --sign -).
"""
from macos_icons import compile_icon, verify_icon

import argparse
from dataclasses import dataclass
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

MAX_BINARY = 1024 ** 3
MAX_HEADER = 1024 ** 2
MAX_ENTRIES = 64
MAX_ARCHIVE_BYTES = MAX_BINARY + 16 * 1024 ** 2
ROOT = Path(__file__).resolve().parents[2]


@dataclass(frozen=True)
class Target:
    system: str
    arch: str
    machine: int
    deb: str = ""


TARGETS = {
    "aarch64-apple-darwin": Target("Darwin", "aarch64", 0x100000C),
    "x86_64-apple-darwin": Target("Darwin", "x86_64", 0x1000007),
    "i686-pc-windows-msvc": Target("Windows", "i686", 0x14C),
    "aarch64-pc-windows-msvc": Target("Windows", "aarch64", 0xAA64),
    "x86_64-unknown-linux-gnu": Target("Linux", "x86_64", 62, "amd64"),
    "aarch64-unknown-linux-gnu": Target("Linux", "aarch64", 183, "arm64"),
}


def require(value, message):
    if not value:
        raise ValueError(message)


def regular(path, maximum=MAX_BINARY):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= maximum,
            "Expected bounded nonempty regular file: " + str(path))
    return info.st_size


def no_symlinks(path):
    path = Path(os.path.abspath(path))
    for item in (path, *path.parents):
        require(not item.is_symlink(), "Symlink path forbidden: " + str(item))
    return path


def run(*args, cwd=None):
    return subprocess.check_output([str(a) for a in args], cwd=cwd,
                                   text=True, timeout=600).strip()


def validate_version(root, version):
    require(re.fullmatch(r"(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})\.(0|[1-9][0-9]{0,4})", version),
            "Version must be a numeric X.Y.Z without path or prerelease characters")
    require(all(int(n) <= 65535 for n in version.split(".")), "Version component exceeds installer limit")
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    require(manifest["workspace"]["package"]["version"] == version,
            "Package version must equal workspace version")


def binary_header(data, target):
    """Inspect bounded headers only; exact thin architecture, not a filename guess."""
    if target.system == "Darwin":
        require(len(data) >= 32 and data[:4] == b"\xcf\xfa\xed\xfe", "Expected thin little-endian Mach-O64")
        require(struct.unpack_from("<I", data, 4)[0] == target.machine,
                "Mach-O architecture mismatch")
        require(struct.unpack_from("<I", data, 12)[0] == 2, "Expected Mach-O executable")
    elif target.system == "Linux":
        require(len(data) >= 64 and data[:7] == b"\x7fELF\x02\x01\x01", "Expected ELF64 little-endian")
        require(struct.unpack_from("<H", data, 18)[0] == target.machine, "ELF architecture mismatch")
        require(struct.unpack_from("<H", data, 16)[0] in (2, 3), "Expected ELF executable or PIE")
        require(struct.unpack_from("<I", data, 20)[0] == 1, "Invalid ELF version")
    else:
        require(len(data) >= 64 and data[:2] == b"MZ", "Expected DOS/PE header")
        offset = struct.unpack_from("<I", data, 60)[0]
        require(64 <= offset <= MAX_HEADER - 26 and offset + 26 <= len(data), "PE header offset out of bounds")
        require(data[offset:offset + 4] == b"PE\0\0", "Expected PE signature")
        machine, sections = struct.unpack_from("<HH", data, offset + 4)
        optional_size, flags = struct.unpack_from("<HH", data, offset + 20)
        magic = 0x10B if target.arch == "i686" else 0x20B
        require(machine == target.machine and struct.unpack_from("<H", data, offset + 24)[0] == magic,
                "PE architecture mismatch")
        require(sections > 0 and optional_size >= (96 if magic == 0x10B else 112)
                and offset + 24 + optional_size <= len(data)
                and flags & 2 and not flags & 0x2000, "Expected PE executable, not DLL")


def verify_binary(path, target):
    regular(path)
    with path.open("rb") as source:
        binary_header(source.read(MAX_HEADER), target)


def executable(root, triple):
    target = TARGETS[triple]
    return root / "target" / triple / "release" / ("iceon.exe" if target.system == "Windows" else "iceon")


def build(root, triple):
    run("cargo", "build", "--locked", "--release", "-p", "iceon-app", "--target", triple, cwd=root)
    verify_binary(no_symlinks(executable(root, triple)), TARGETS[triple])


def archive_name(name):
    path = PurePosixPath(name)
    require(name and not path.is_absolute() and "\\" not in name
            and all(part not in ("", ".", "..") for part in name.split("/"))
            and ":" not in name and "\x00" not in name, "Unsafe archive path")


def verify_archive(path, target, payload_name, zipped=False):
    """Never extract untrusted archives; bound metadata, total size and header reads."""
    regular(path, MAX_ARCHIVE_BYTES)
    if zipped:
        # ZipFile reads the entire central directory before exposing entries.
        # Bound that allocation before constructing it; ZIP64 is unnecessary
        # for our < 1 GiB, <= 64-entry preparation artifacts.
        with path.open("rb") as source:
            source.seek(max(0, path.stat().st_size - 65557))
            tail = source.read(65557)
        end = tail.rfind(b"PK\x05\x06")
        require(end >= 0 and end + 22 <= len(tail), "Missing ZIP end record")
        disk, directory_disk, on_disk, count, directory_size, _, comment = struct.unpack_from("<HHHHIIH", tail, end + 4)
        require(disk == directory_disk == 0 and on_disk == count
                and count <= MAX_ENTRIES and directory_size <= MAX_HEADER
                and end + 22 + comment == len(tail), "ZIP directory limits exceeded")
    seen = set()
    total = 0
    with (zipfile.ZipFile(path) if zipped else tarfile.open(path, "r:gz")) as archive:
        entries = archive.infolist() if zipped else archive
        for entry in entries:
            require(len(seen) < MAX_ENTRIES, "Too many archive entries")
            name = entry.filename if zipped else entry.name
            archive_name(name)
            require(name not in seen, "Duplicate archive entry")
            seen.add(name)
            if zipped:
                require(not entry.is_dir() and not entry.flag_bits & 1
                        and stat.S_IFMT(entry.external_attr >> 16) in (0, stat.S_IFREG), "Unsafe ZIP member")
                size = entry.file_size
            else:
                require(entry.isfile() or entry.isdir(), "Archive links and special files forbidden")
                size = entry.size
                require(size == 0 or entry.isfile(), "Non-file archive data")
            total += size
            require(0 <= size <= MAX_BINARY and total <= MAX_ARCHIVE_BYTES, "Archive size limit exceeded")
            if name == payload_name:
                if not zipped:
                    require(entry.isfile() and entry.mode & 0o111, "Archive payload must be executable")
                with (archive.open(entry) if zipped else archive.extractfile(entry)) as source:
                    binary_header(source.read(MAX_HEADER), target)
        require(payload_name in seen, "Archive executable missing")


def add_tar(source, output, arcname):
    with tarfile.open(output, "w:gz") as archive:
        archive.add(source, arcname=arcname, recursive=True)


def package_mac(root, stage, out, exe, target, version, stem):
    app = stage / "Iceon.app"
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    shutil.copy2(exe, contents / "MacOS/iceon")
    (contents / "MacOS/iceon").chmod(0o755)
    icon = no_symlinks(root / "assets/icons/icon.icns")
    regular(icon, 16 * 1024 ** 2)
    shutil.copy2(icon, contents / "Resources/icon.icns")
    info = dict(CFBundleName="Iceon", CFBundleDisplayName="Iceon", CFBundleExecutable="iceon",
                CFBundleIdentifier="ad.yan.iceon", CFBundlePackageType="APPL",
                CFBundleShortVersionString=version, CFBundleVersion=version,
                CFBundleIconFile="icon.icns", NSHighResolutionCapable=True,
                CFBundleURLTypes=[dict(CFBundleURLName="ad.yan.iceon.actions",
                                      CFBundleTypeRole="Viewer", CFBundleURLSchemes=["iceon"])])
    compile_icon(root, contents / "Resources", info)
    (contents / "Info.plist").write_bytes(plistlib.dumps(info))
    verify_icon(contents / "Resources", info)
    # Integrity only, required by native ARM64; never Developer ID or notarization.
    run("codesign", "--force", "--sign", "-", app)
    run("codesign", "--verify", "--deep", "--strict", app)
    verify_binary(contents / "MacOS/iceon", target)
    add_tar(app, out / (stem + ".app.tar.gz"), "Iceon.app")
    verify_archive(out / (stem + ".app.tar.gz"), target, "Iceon.app/Contents/MacOS/iceon")
    (stage / "Applications").symlink_to("/Applications")
    run("hdiutil", "create", "-volname", "Iceon " + version, "-srcfolder", stage,
        "-format", "UDZO", out / (stem + ".dmg"))
    run("hdiutil", "verify", out / (stem + ".dmg"))


def package_windows(root, out, exe, target, version, stem):
    installer = "iceon-arm64.nsi" if target.arch == "aarch64" else "iceon.nsi"
    prefix = "/" if os.name == "nt" else "-"
    run(os.environ.get("MAKENSIS", "makensis"), prefix + "WX", prefix + "V3",
        prefix + "DVERSION=" + version, prefix + "DPAYLOAD=" + str(exe),
        prefix + "DOUTPUT=" + str(out / (stem + "-setup.exe")),
        prefix + "DLICENSE_FILE=" + str(no_symlinks(root / "LICENSE")),
        prefix + "DICON_FILE=" + str(no_symlinks(root / "assets/icons/icon.ico")),
        no_symlinks(root / ".github/installer" / installer))
    # NSIS bootstrap is x86 even when its installed payload is ARM64.
    verify_binary(out / (stem + "-setup.exe"), TARGETS["i686-pc-windows-msvc"])
    with zipfile.ZipFile(out / (stem + ".zip"), "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(exe, "Iceon/iceon.exe")
        archive.write(no_symlinks(root / "LICENSE"), "Iceon/LICENSE")
    verify_archive(out / (stem + ".zip"), target, "Iceon/iceon.exe", zipped=True)


def shlib_dependencies(stage, binary):
    # A real native Debian database and matching runtime -dev/library packages are
    # required. Missing shlibs/symbols is an error, never a guessed dependency list.
    debian = stage / "debian"
    debian.mkdir()
    (debian / "control").write_text("Source: iceon\nSection: utils\nPriority: optional\nMaintainer: Iceon contributors <noreply@iceon.dev>\n\nPackage: iceon\nArchitecture: any\nDescription: Native Kubernetes workspace\n")
    output = run("dpkg-shlibdeps", "--warnings=7", "-O", "-e" + str(binary), cwd=stage)
    lines = output.splitlines()
    require(len(lines) == 1 and lines[0].startswith("shlibs:Depends="), "Missing dpkg-shlibdeps result")
    deps = lines[0].removeprefix("shlibs:Depends=")
    require(deps and len(deps) <= 16384 and not any(c in deps for c in "\r\n\x00"), "Invalid shlibs dependencies")
    return deps


def package_linux(root, stage, out, exe, target, version, stem):
    require(run("dpkg", "--print-architecture") == target.deb, "Matching native Debian architecture required")
    tree = stage / "Iceon"
    (tree / "bin").mkdir(parents=True)
    shutil.copy2(exe, tree / "bin/iceon")
    (tree / "bin/iceon").chmod(0o755)
    shutil.copy2(no_symlinks(root / "LICENSE"), tree / "LICENSE")
    add_tar(tree, out / (stem + ".tar.gz"), "Iceon")
    verify_archive(out / (stem + ".tar.gz"), target, "Iceon/bin/iceon")
    package = stage / "deb"
    (package / "usr/bin").mkdir(parents=True)
    shutil.copy2(tree / "bin/iceon", package / "usr/bin/iceon")
    deps = shlib_dependencies(stage, package / "usr/bin/iceon")
    (package / "DEBIAN").mkdir()
    (package / "DEBIAN/control").write_text(
        f"Package: iceon\nVersion: {version}\nArchitecture: {target.deb}\n"
        f"Maintainer: Iceon contributors <noreply@iceon.dev>\nDepends: {deps}\n"
        "Section: utils\nPriority: optional\nDescription: Native Kubernetes workspace\n")
    for source_name, destination in (
        ("ad.yan.iceon.desktop", "usr/share/applications/ad.yan.iceon.desktop"),
        ("ad.yan.iceon.metainfo.xml", "usr/share/metainfo/ad.yan.iceon.metainfo.xml"),
    ):
        source = root / "packaging/linux" / source_name
        # AppStream accepts both suffixes; use the shared Flatpak metadata,
        # never duplicate a second desktop/application identity here.
        if source_name.endswith(".metainfo.xml") and not source.exists():
            source = source.with_name("ad.yan.iceon.appdata.xml")
        source = no_symlinks(source)
        regular(source, 1024 ** 2)
        dest = package / destination
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
    icon = no_symlinks(root / "assets/icons/icon.png")
    regular(icon, 16 * 1024 ** 2)
    dest = package / "usr/share/icons/hicolor/1024x1024/apps/ad.yan.iceon.png"
    dest.parent.mkdir(parents=True)
    shutil.copy2(icon, dest)
    license_dest = package / "usr/share/doc/iceon/copyright"
    license_dest.parent.mkdir(parents=True)
    shutil.copy2(tree / "LICENSE", license_dest)
    output = out / f"iceon_{version}_{target.deb}.deb"
    run("dpkg-deb", "--build", "--root-owner-group", package, output)
    for field, expected in (("Package", "iceon"), ("Version", version),
                            ("Architecture", target.deb), ("Depends", deps)):
        require(run("dpkg-deb", "--field", output, field) == expected, "Debian field mismatch: " + field)


def package(root, triple, version, output):
    target = TARGETS[triple]
    validate_version(root, version)
    require(platform.system() == target.system, "Package on the target operating system")
    exe = no_symlinks(executable(root, triple))
    verify_binary(exe, target)
    regular(no_symlinks(root / "LICENSE"), 1024 ** 2)
    out = no_symlinks(output)
    out.mkdir(parents=True, exist_ok=True)
    require(not any(out.iterdir()), "Output directory must be empty")
    stem = f"Iceon_{version}_{target.arch}"
    # Publish to the requested directory only after all preparation succeeds.
    with tempfile.TemporaryDirectory(prefix="iceon-distribution-") as temporary:
        stage = Path(temporary) / "stage"
        artifacts = Path(temporary) / "artifacts"
        stage.mkdir()
        artifacts.mkdir()
        if target.system == "Darwin":
            package_mac(root, stage, artifacts, exe, target, version, stem)
        elif target.system == "Windows":
            package_windows(root, artifacts, exe, target, version, stem)
        else:
            package_linux(root, stage, artifacts, exe, target, version, stem)
        require(not any(out.iterdir()), "Output directory changed during packaging")
        for artifact in artifacts.iterdir():
            regular(artifact, MAX_ARCHIVE_BYTES)
            # Exclusive create refuses collisions rather than replacing user files.
            with artifact.open("rb") as source, (out / artifact.name).open("xb") as dest:
                shutil.copyfileobj(source, dest)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("build", "package"):
        command = commands.add_parser(name)
        command.add_argument("--target", choices=TARGETS, required=True)
        if name == "package":
            command.add_argument("--version", required=True)
            command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(ROOT, args.target)
        else:
            package(ROOT, args.target, args.version, args.output)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f"distribution preparation failed: {error}\n")
    print("DISTRIBUTION_PREPARATION_OK")


if __name__ == "__main__":
    main()

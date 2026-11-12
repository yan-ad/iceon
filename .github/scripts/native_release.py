#!/usr/bin/env python3
"""Native 0.1.3 packaging and fail-closed publication (Python 3.11+, stdlib)."""
import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import tarfile
import tempfile
import time
import tomllib
import zipfile

REPOSITORY = "yan-ad/iceon"
VERSION = "0.1.3"
PACKAGES = ("iceon-app", "iceon-core", "iceon-ui", "iceon-xtask")
MAC = f"Iceon_{VERSION}_aarch64.app.tar.gz"
DMG = f"Iceon_{VERSION}_aarch64.dmg"
WIN = f"Iceon_{VERSION}_x86-setup.exe"
PORTABLE = f"Iceon_{VERSION}_i686.zip"
FEED = "latest.json"
SUMS = "SHASUMS256.txt"
NAMES = {MAC, DMG, WIN, PORTABLE, MAC + ".sig", WIN + ".sig", FEED, SUMS}
PLATFORMS = {"darwin-aarch64": MAC, "darwin-aarch64-app": MAC,
             "windows-i686": WIN, "windows-i686-nsis": WIN}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def run(*args):
    return subprocess.check_output(args, text=True, timeout=300).strip()


def verify_dmg(path):
    # DiskImages/Spotlight can briefly retain a newly created image on CI hosts.
    # Retry only explicit transient locks, never malformed images or bad checksums.
    for attempt in range(5):
        try:
            return subprocess.check_output(
                ("hdiutil", "verify", str(path)), text=True,
                stderr=subprocess.STDOUT, timeout=300,
            ).strip()
        except subprocess.CalledProcessError as error:
            output = error.output or ""
            print(output.rstrip(), flush=True)
            transient = any(message in output.lower() for message in
                            ("resource temporarily unavailable", "resource busy"))
            if not transient or attempt == 4:
                raise
            delay = 2 ** (attempt + 1)
            print(f"DMG temporarily busy; retrying verification in {delay}s", flush=True)
            time.sleep(delay)


def changelog(root):
    text = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    matches = list(re.finditer(r"^## " + re.escape(VERSION) + r"(?: \(pending CI\))?\s*$", text, re.M))
    require(len(matches) == 1, "Need exactly one canonical 0.1.3 changelog section")
    body = re.split(r"^## ", text[matches[0].end():], maxsplit=1, flags=re.M)[0].strip()
    require(bool(body), "Empty release changelog")
    return body


def versions(root):
    workspace = tomllib.loads((root / "Cargo.toml").read_text())
    require(workspace["workspace"]["package"]["version"] == VERSION, "Workspace version must be 0.1.3")
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    for name in PACKAGES:
        manifest = tomllib.loads((root / "crates" / name / "Cargo.toml").read_text())["package"]
        require(manifest["name"] == name, "Package identity mismatch")
        require(manifest["version"] == VERSION or manifest["version"] == {"workspace": True}, "Package version mismatch")
        entries = [p for p in lock["package"] if p["name"] == name]
        require(len(entries) == 1 and entries[0]["version"] == VERSION and "source" not in entries[0], "Local lock package mismatch: " + name)


def remote_tag(root, sha, expected=None):
    # Query only the canonical remote, including peeled annotated tags.
    lines = run("git", "-C", str(root), "ls-remote", "https://github.com/" + REPOSITORY + ".git", "refs/tags/" + VERSION, "refs/tags/" + VERSION + "^{}")
    refs = dict(line.split()[::-1] for line in lines.splitlines())
    raw = refs.get("refs/tags/" + VERSION)
    require(raw and re.fullmatch(r"[0-9a-f]{40}", raw), "Existing exact remote tag required")
    require(refs.get("refs/tags/" + VERSION + "^{}", raw) == sha, "Remote tag does not point to workflow commit")
    require(expected is None or raw == expected, "Remote tag object changed during release")
    return raw


def guard(root, expected=None):
    require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "Canonical repository required")
    require(os.environ.get("RELEASE_TAG") == VERSION, "Only exact tag 0.1.3 is accepted")
    event = os.environ.get("GITHUB_EVENT_NAME")
    require(event in ("push", "workflow_dispatch"), "Unsupported release event")
    if event == "push":
        require(os.environ.get("GITHUB_REF") == "refs/tags/" + VERSION, "Exact tag push required")
    sha = os.environ.get("GITHUB_SHA", "")
    require(re.fullmatch(r"[0-9a-f]{40}", sha), "Invalid Actions commit")
    require(run("git", "-C", str(root), "rev-parse", "HEAD") == sha, "Checkout must equal Actions/CI commit")
    require(run("git", "-C", str(root), "rev-parse", "refs/tags/" + VERSION + "^{commit}") == sha, "Local tag mismatch")
    versions(root)
    changelog(root)
    return remote_tag(root, sha, expected)


def fingerprint(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and 0 < info.st_size <= 1024 ** 3, "Expected bounded nonempty regular file: " + str(path))
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return info.st_size, digest.hexdigest()


def binary(data, platform):
    if platform == "mac":
        require(len(data) >= 32 and data[:4] == b"\xcf\xfa\xed\xfe" and struct.unpack_from("<I", data, 4)[0] == 0x100000C, "Expected thin ARM64 Mach-O")
    else:
        require(len(data) >= 64 and data[:2] == b"MZ", "Missing DOS header")
        offset = struct.unpack_from("<I", data, 60)[0]
        require(offset + 26 <= len(data) and data[offset:offset + 4] == b"PE\0\0", "Missing PE header")
        require(struct.unpack_from("<H", data, offset + 4)[0] == 0x14C and struct.unpack_from("<H", data, offset + 24)[0] == 0x10B, "Expected i686 PE32")


def verify_bundle(app):
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    require(info["CFBundleIdentifier"] == "ad.yan.iceon" and info["CFBundleExecutable"] == "iceon" and info["CFBundlePackageType"] == "APPL", "Incorrect bundle identity")
    require(info["CFBundleShortVersionString"] == VERSION and info["CFBundleVersion"] == VERSION, "Incorrect bundle version")
    exe = app / "Contents/MacOS/iceon"
    binary(exe.read_bytes(), "mac")
    require(exe.stat().st_mode & 0o111, "Bundle executable lost permissions")
    if "CFBundleIconFile" in info:
        require((app / "Contents/Resources" / info["CFBundleIconFile"]).is_file(), "Missing bundle icon")
    run("codesign", "--verify", "--deep", "--strict", str(app))
    require("Signature=adhoc" in subprocess.run(["codesign", "-dv", str(app)], capture_output=True, text=True, check=True).stderr, "Expected ad-hoc signature")


def package(root, out, platform):
    out.mkdir(parents=True, exist_ok=True)
    require(not list(out.iterdir()), "Packaging directory must be empty")
    if platform == "windows":
        exe = root / "target/i686-pc-windows-msvc/release/iceon.exe"
        binary(exe.read_bytes(), "windows")
        compiler = os.environ.get("MAKENSIS", "makensis")
        prefix = "/" if os.name == "nt" else "-"
        run(compiler, prefix + "WX", prefix + "V3", prefix + "DVERSION=" + VERSION,
            prefix + "DPAYLOAD=" + str(exe), prefix + "DOUTPUT=" + str(out / WIN),
            prefix + "DLICENSE_FILE=" + str(root / "LICENSE"),
            prefix + "DICON_FILE=" + str(root / "assets/icons/icon.ico"),
            str(root / ".github/installer/iceon.nsi"))
        verify_installer(out / WIN)
        with zipfile.ZipFile(out / PORTABLE, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.write(exe, "Iceon/iceon.exe")
        verify_portable(out / PORTABLE)
        return
    require(run("uname", "-m") == "arm64", "Native ARM64 macOS runner required")
    with tempfile.TemporaryDirectory() as temporary:
        stage = Path(temporary) / "image"
        app = stage / "Iceon.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        (app / "Contents/Resources").mkdir()
        shutil.copy2(root / "target/aarch64-apple-darwin/release/iceon", app / "Contents/MacOS/iceon")
        (app / "Contents/MacOS/iceon").chmod(0o755)
        info = dict(CFBundleName="Iceon", CFBundleDisplayName="Iceon", CFBundleExecutable="iceon", CFBundleIdentifier="ad.yan.iceon", CFBundlePackageType="APPL", CFBundleShortVersionString=VERSION, CFBundleVersion=VERSION, NSHighResolutionCapable=True)
        icon = root / "assets/icons/icon.icns"
        if icon.is_file():
            shutil.copy2(icon, app / "Contents/Resources/icon.icns")
            info["CFBundleIconFile"] = "icon.icns"
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        run("codesign", "--force", "--sign", "-", str(app))
        verify_bundle(app)
        with tarfile.open(out / MAC, "w:gz") as archive:
            archive.add(app, arcname="Iceon.app")
        (stage / "Applications").symlink_to("/Applications")
        run("hdiutil", "create", "-volname", "Iceon " + VERSION, "-srcfolder", str(stage), "-format", "UDZO", str(out / DMG))
        verify_dmg(out / DMG)
        mounted = Path(temporary) / "mounted"
        run("hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", str(mounted), str(out / DMG))
        try:
            verify_bundle(mounted / "Iceon.app")
            require((mounted / "Applications").is_symlink() and os.readlink(mounted / "Applications") == "/Applications", "Missing Applications link")
            require(fingerprint(mounted / "Iceon.app/Contents/MacOS/iceon") == fingerprint(app / "Contents/MacOS/iceon"), "DMG binary changed")
        finally:
            run("hdiutil", "detach", str(mounted))
    verify_tar(out / MAC)


def verify_installer(path):
    # The Windows runner separately installs it and checks the installed payload hash.
    binary(path.read_bytes(), "windows")


def update_signature(path):
    fingerprint(path)
    require(path.stat().st_size <= 16 * 1024, "Signature too large")
    encoded = path.read_text(encoding="ascii").strip()
    try:
        lines = base64.b64decode(encoded, validate=True).decode("utf-8").splitlines()
    except (ValueError, UnicodeError) as error:
        raise ValueError("Invalid updater signature encoding") from error
    require(len(lines) == 4 and lines[0].startswith("untrusted comment: ")
            and lines[2].startswith("trusted comment: "), "Invalid signature document")
    fields = lines[2].removeprefix("trusted comment: ").split("\t")
    require([f for f in fields if f.startswith("version:")] == ["version:" + VERSION],
            "Signature must authenticate the exact release version")
    return encoded


def verify_updates(directory):
    for name in (MAC, WIN):
        update_signature(directory / (name + ".sig"))
        run("cargo", "xtask", "verify-update", str(directory / (name + ".sig")),
            str(directory / name), VERSION)


def updater_manifest(directory, create=False):
    verify_updates(directory)
    platforms = {key: {"url": f"https://github.com/{REPOSITORY}/releases/download/{VERSION}/{name}",
                       "signature": update_signature(directory / (name + ".sig"))}
                 for key, name in PLATFORMS.items()}
    path = directory / FEED
    if create:
        require(not path.exists(), "Never overwrite updater manifest")
        manifest = {"version": VERSION, "notes": "Native GPUI migration; future native updates require manual installation until a native updater is integrated.",
                    "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "platforms": platforms}
        with path.open("x", encoding="utf-8") as output:
            output.write(json.dumps(manifest, indent=2) + "\n")
    fingerprint(path)
    require(path.stat().st_size <= 64 * 1024, "Updater manifest too large")
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "Duplicate manifest key")
            result[key] = value
        return result
    manifest = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_pairs)
    require(set(manifest) == {"version", "notes", "pub_date", "platforms"}
            and manifest["version"] == VERSION and manifest["platforms"] == platforms,
            "Updater version, platforms, URLs or signatures differ")
    require(isinstance(manifest["notes"], str) and bool(manifest["notes"]), "Missing updater notes")
    require(isinstance(manifest["pub_date"], str)
            and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", manifest["pub_date"]), "Invalid publication date")
    datetime.strptime(manifest["pub_date"], "%Y-%m-%dT%H:%M:%SZ")


def sign_updates(directory):
    require({p.name for p in directory.iterdir()} == {MAC, DMG, WIN, PORTABLE}, "Expected only final unsigned update payloads")
    for name in (MAC, WIN):
        run("cargo", "xtask", "sign-update", str(directory / name), VERSION)
    updater_manifest(directory, create=True)


def verify_tar(path):
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        names = [m.name for m in members]
        require(len(names) == len(set(names)), "Duplicate tar members")
        require(all((m.name == "Iceon.app" or m.name.startswith("Iceon.app/")) and ".." not in m.name.split("/") and (m.isfile() or m.isdir()) and m.size <= 1024 ** 3 for m in members), "Unsafe tar member")
        for name in ("Iceon.app/Contents/Info.plist", "Iceon.app/Contents/MacOS/iceon", "Iceon.app/Contents/_CodeSignature/CodeResources"):
            require(name in names and archive.getmember(name).isfile(), "Missing bundle member: " + name)
        info = plistlib.loads(archive.extractfile("Iceon.app/Contents/Info.plist").read())
        require(info["CFBundleVersion"] == VERSION and info["CFBundleShortVersionString"] == VERSION and info["CFBundleExecutable"] == "iceon" and info["CFBundleIdentifier"] == "ad.yan.iceon", "Tar bundle metadata mismatch")
        member = archive.getmember("Iceon.app/Contents/MacOS/iceon")
        require(member.mode & 0o111, "Tar executable permission missing")
        binary(archive.extractfile(member).read(), "mac")


def verify_portable(path):
    with zipfile.ZipFile(path) as archive:
        require(archive.namelist() == ["Iceon/iceon.exe"], "Unexpected portable ZIP entries")
        info = archive.infolist()[0]
        require(0 < info.file_size <= 1024 ** 3, "Invalid portable ZIP member size")
        require(not stat.S_ISLNK(info.external_attr >> 16) and not info.flag_bits & 1, "Linked/encrypted portable ZIP member")
        require(not archive.testzip(), "Portable ZIP integrity failed")
        binary(archive.read(info), "windows")


def checksums(directory, create=False):
    expected_names = NAMES - {SUMS} if create else NAMES
    require({p.name for p in directory.iterdir()} == expected_names, "Unexpected/missing release assets")
    fingerprints = {name: fingerprint(directory / name) for name in sorted(expected_names)}
    text = "".join(f"{fingerprints[name][1]}  {name}\n" for name in sorted(NAMES - {SUMS}))
    if create:
        (directory / SUMS).write_text(text, encoding="ascii")
    else:
        require((directory / SUMS).read_text(encoding="ascii") == text, "Checksum mismatch")
    verify_tar(directory / MAC)
    verify_installer(directory / WIN)
    verify_portable(directory / PORTABLE)
    updater_manifest(directory)
    require((directory / DMG).stat().st_size >= 512, "DMG too small")
    with (directory / DMG).open("rb") as source:
        source.seek(-512, 2)
        require(source.read(4) == b"koly", "Expected UDIF DMG trailer")


def api(path, *args):
    return json.loads(run("gh", "api", "repos/" + REPOSITORY + "/" + path, *args))


def no_release():
    # Enumerate drafts too; unlike a tag lookup, API errors cannot masquerade as absence.
    pages = json.loads(run("gh", "api", "--paginate", "--slurp", "repos/" + REPOSITORY + "/releases?per_page=100"))
    require(not any(r["tag_name"] == VERSION for page in pages for r in page), "Release already exists; never resume or overwrite")


def asset_identity(assets):
    # Verification downloads change download_count; ignore telemetry and ordering.
    return sorted((a["id"], a["name"], a["size"], a["state"], a.get("digest")) for a in assets)


def publish(root, directory, tag_object):
    require(bool(tag_object), "Initial immutable tag object required")
    guard(root, tag_object)
    checksums(directory)
    no_release()
    with tempfile.TemporaryDirectory() as temporary:
        notes = Path(temporary) / "notes.md"
        notes.write_text(changelog(root) + "\n\nNative GPUI packages with a Windows per-user setup EXE and signed migration feed for compatible older clients. macOS is ad-hoc signed, not notarized; Windows has no Authenticode signature. The native app does not yet update itself. Pre-updater installations require manual installation; installed-client migration smoke testing remains necessary.\n", encoding="utf-8")
        # The tag endpoint does not reliably expose unpublished drafts. Keep the
        # ID from creation, and pin provenance to the same commit as reusable CI.
        draft = api("releases", "--method", "POST", "-f", "tag_name=" + VERSION,
                    "-f", "target_commitish=" + os.environ["GITHUB_SHA"],
                    "-f", "name=Iceon " + VERSION, "-f", "body=" + notes.read_text(encoding="utf-8"),
                    "-F", "draft=true", "-F", "prerelease=false", "-f", "make_latest=false")
        rid = draft["id"]
        require(type(rid) is int and rid > 0 and draft["tag_name"] == VERSION
                and draft["draft"] and not draft["prerelease"] and not draft["assets"],
                "Expected fresh empty stable draft")
        run("gh", "release", "upload", VERSION, *[str(directory / n) for n in sorted(NAMES)], "--repo", REPOSITORY)
        release = api(f"releases/{rid}")
        require(release["tag_name"] == VERSION and release["draft"] and not release["prerelease"], "Draft changed")
        assets = release["assets"]
        require(len(assets) == len(NAMES) and {a["name"] for a in assets} == NAMES, "Uploaded asset set mismatch")
        # macOS temp directories may start with the /var -> /private/var symlink;
        # the signing verifier intentionally rejects symlink path ancestors.
        downloaded = Path(temporary).resolve() / "downloaded"
        downloaded.mkdir()
        for asset in assets:
            size, digest = fingerprint(directory / asset["name"])
            require(asset["size"] == size and asset["state"] == "uploaded", "Uploaded asset size/state mismatch")
            require(asset.get("digest") in (None, "sha256:" + digest), "GitHub digest mismatch")
            destination = downloaded / asset["name"]
            with destination.open("wb") as output:
                subprocess.run(["gh", "api", "-H", "Accept: application/octet-stream", f"repos/{REPOSITORY}/releases/assets/{asset['id']}"], stdout=output, check=True, timeout=300)
            require(fingerprint(destination) == (size, digest), "Downloaded bytes differ")
        checksums(downloaded)
        guard(root, tag_object)
        # Refuse concurrent draft edits as well as tag movement before publishing.
        current = api(f"releases/{rid}")
        require(current["tag_name"] == VERSION and current["draft"] and not current["prerelease"]
                and asset_identity(current["assets"]) == asset_identity(assets), "Draft changed before publication")
        run("gh", "release", "edit", VERSION, "--repo", REPOSITORY, "--draft=false", "--prerelease=false", "--latest")
        published = api(f"releases/{rid}")
        require(not published["draft"] and not published["prerelease"] and published["tag_name"] == VERSION, "Publication state mismatch")
        require(api("releases/latest")["id"] == rid, "Latest release identity mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["guard", "package-mac", "package-windows", "sign-updates", "checksums", "publish"])
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--directory", type=Path, default=Path("dist"))
    parser.add_argument("--tag-object")
    args = parser.parse_args()
    if args.command == "guard":
        raw = guard(args.root, args.tag_object)
        no_release()
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
                output.write("tag_object=" + raw + "\n")
    elif args.command.startswith("package-"):
        package(args.root.resolve(), args.directory.resolve(), "mac" if args.command == "package-mac" else "windows")
    elif args.command == "sign-updates":
        sign_updates(args.directory)
    elif args.command == "checksums":
        checksums(args.directory, create=True)
    else:
        publish(args.root, args.directory, args.tag_object)


if __name__ == "__main__":
    main()

"""Compile Icon Composer assets at packaging time; never edit source documents.

Requires macOS 26+ and Xcode 26+ (Command Line Tools and ictool PNG exports are
not sufficient).
actool owns the icon schema and emitted Info.plist metadata. See Apple's Creating
Your App Icon Using Icon Composer and the actool(1) --output-partial-info-plist
contract. A missing compiler/asset is an error, not a silent legacy-only package.
"""
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile


def check_toolchain():
    """Reject incompatible hosts and tools before starting the asset runtime."""
    host = platform.mac_ver()[0]
    print(f"Icon Composer host macOS: {host or 'unknown'}", flush=True)
    # Xcode 26.3's bundled asset runtime can crash with missing CoreMedia /
    # MediaToolbox symbols on macOS 15 even though actool --version succeeds.
    # Keep the bundled-runtime mode, but require the matching OS generation.
    if not re.fullmatch(r"\d+(?:\.\d+)*", host) or int(host.split(".")[0]) < 26:
        raise ValueError("Icon Composer packaging requires a macOS 26+ host; use macos-26 or macos-26-intel")

    def query(args):
        return subprocess.run(args, check=True, capture_output=True, timeout=30).stdout

    compiler = query(["xcrun", "--find", "actool"]).decode().strip()
    xcode = query(["xcrun", "xcodebuild", "-version"]).decode().strip()
    print(f"Icon Composer toolchain: {compiler}\n{xcode}", flush=True)
    match = re.search(r"^Xcode (\d+)(?:\.|$)", xcode, re.MULTILINE)
    if not match or int(match[1]) < 26:
        raise ValueError("Icon Composer requires Xcode 26+; select a full Xcode with DEVELOPER_DIR")
    version_info = plistlib.loads(query(["xcrun", "actool", "--output-format=xml1", "--version"]))
    version = version_info.get("com.apple.actool.version", {}).get("bundle-version", "")
    print(f"Icon Composer actool bundle-version: {version}", flush=True)
    # 24127 is the shipping Xcode 26.0 actool, not Xcode's marketing version.
    if not isinstance(version, str) or not re.fullmatch(r"\d+(?:\.\d+)*", version):
        raise ValueError("Cannot determine actool bundle-version; Xcode 26+ is required")
    if tuple(map(int, version.split("."))) < (24127,):
        raise ValueError(f"Icon Composer requires actool 24127+ (Xcode 26+); found {version}")


def compile_icon(root, resources, info):
    source = Path(root) / "assets/icons/iceon.icon"
    if not (source / "icon.json").is_file():
        raise ValueError("Missing Icon Composer document")
    check_toolchain()
    with tempfile.TemporaryDirectory(prefix="iceon-icons-") as temporary:
        output = Path(temporary)
        partial = output / "icon-info.plist"
        subprocess.run([
            "xcrun", "actool", str(source), "--compile", str(output),
            "--output-format", "human-readable-text", "--notices", "--warnings", "--errors",
            # Use bundled frameworks for reproducible output on supported hosts.
            "--lightweight-asset-runtime-mode", "enabled",
            "--app-icon", source.stem, "--platform", "macosx",
            "--include-all-app-icons", "--enable-on-demand-resources", "NO",
            "--target-device", "mac", "--minimum-deployment-target", "11.0",
            "--output-partial-info-plist", str(partial),
        ], check=True, timeout=300)
        metadata = plistlib.loads(partial.read_bytes())
        if metadata.get("CFBundleIconName") != source.stem:
            raise ValueError("Compiler did not declare the primary Icon Composer icon")
        catalog = output / "Assets.car"
        if not catalog.is_file() or not catalog.stat().st_size:
            raise ValueError("Compiler did not produce Assets.car")
        # Do not allow generated metadata to replace bundle identity/version keys.
        unexpected = set(metadata) - {"CFBundleIconName", "CFBundleIconFile", "CFBundleIcons", "CFBundleIcons~ipad"}
        if unexpected:
            raise ValueError("Unexpected icon metadata keys: " + str(sorted(unexpected)))
        for item in output.iterdir():
            if item != partial and item.is_file():
                shutil.copy2(item, Path(resources) / item.name)
        # Keep the existing ICNS artwork for older macOS releases. The named
        # compiled icon takes precedence on systems supporting Icon Composer.
        metadata.pop("CFBundleIconFile", None)
        info.update(metadata)
        verify_icon(resources, info)


def verify_icon(resources, info):
    resources = Path(resources)
    if info.get("CFBundleIconName") != "iceon":
        raise ValueError("Missing Icon Composer primary icon registration")
    catalog = resources / "Assets.car"
    if not catalog.is_file() or not catalog.stat().st_size:
        raise ValueError("Missing compiled Icon Composer Assets.car")
    fallback = info.get("CFBundleIconFile")
    if not isinstance(fallback, str) or Path(fallback).name != fallback:
        raise ValueError("Missing or invalid fallback icon registration")
    if not (resources / fallback).is_file():
        raise ValueError("Missing fallback icon file")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-toolchain", action="store_true", required=True)
    parser.parse_args()
    check_toolchain()

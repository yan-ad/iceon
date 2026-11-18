"""Compile Icon Composer assets at packaging time; never edit source documents.

Requires Xcode 26+ (Command Line Tools and ictool PNG exports are not sufficient).
actool owns the icon schema and emitted Info.plist metadata. See Apple's Creating
Your App Icon Using Icon Composer and the actool(1) --output-partial-info-plist
contract. A missing compiler/asset is an error, not a silent legacy-only package.
"""
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile


def compile_icon(root, resources, info):
    source = Path(root) / "assets/icons/iceon.icon"
    if not (source / "icon.json").is_file():
        raise ValueError("Missing Icon Composer document")
    with tempfile.TemporaryDirectory(prefix="iceon-icons-") as temporary:
        output = Path(temporary)
        partial = output / "icon-info.plist"
        subprocess.run([
            "xcrun", "actool", str(source), "--compile", str(output),
            "--app-icon", source.stem, "--platform", "macosx",
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

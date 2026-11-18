"""Isolated packaging regressions; no Xcode or source/user file writes."""
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import macos_icons as icons


class MacIconTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "assets/icons/iceon.icon"
        self.source.mkdir(parents=True)
        self.document = self.source / "icon.json"
        self.document.write_text('{"fill":{"automatic-gradient":"fixture"}}')
        self.original = self.document.read_bytes()
        self.resources = self.root / "Resources"
        self.resources.mkdir()
        (self.resources / "icon.icns").write_bytes(b"legacy")
        self.info = {"CFBundleIconFile": "icon.icns", "CFBundleIdentifier": "ad.yan.iceon"}

    def compiler(self, args, **kwargs):
        self.assertEqual(args[:2], ["xcrun", "actool"])
        self.assertIn(str(self.source), args)
        self.assertEqual(args[args.index("--platform") + 1], "macosx")
        self.assertEqual(args[args.index("--app-icon") + 1], "iceon")
        output = Path(args[args.index("--compile") + 1])
        (output / "Assets.car").write_bytes(b"compiled-appearance-asset")
        partial = Path(args[args.index("--output-partial-info-plist") + 1])
        partial.write_bytes(plistlib.dumps({"CFBundleIconName": "iceon", "CFBundleIconFile": "iceon.icns"}))

    def test_compiler_metadata_and_catalog_are_packaged_without_editing_source(self):
        with patch.object(icons.subprocess, "run", side_effect=self.compiler):
            icons.compile_icon(self.root, self.resources, self.info)
        icons.verify_icon(self.resources, self.info)
        self.assertEqual(self.info["CFBundleIdentifier"], "ad.yan.iceon")
        self.assertEqual(self.document.read_bytes(), self.original)
        self.assertEqual((self.resources / "icon.icns").read_bytes(), b"legacy")
        self.assertFalse((self.resources / "icon-info.plist").exists())

    def test_missing_tool_does_not_silently_package_only_legacy_icon(self):
        with patch.object(icons.subprocess, "run", side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                icons.compile_icon(self.root, self.resources, self.info)
        self.assertNotIn("CFBundleIconName", self.info)
        self.assertEqual(self.document.read_bytes(), self.original)

    def test_failed_compile_propagates(self):
        with patch.object(icons.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "actool")):
            with self.assertRaises(subprocess.CalledProcessError):
                icons.compile_icon(self.root, self.resources, self.info)

    def test_missing_compiled_catalog_is_rejected(self):
        def missing(args, **kwargs):
            self.compiler(args, **kwargs)
            (Path(args[args.index("--compile") + 1]) / "Assets.car").unlink()
        with patch.object(icons.subprocess, "run", side_effect=missing):
            with self.assertRaisesRegex(ValueError, "Assets.car"):
                icons.compile_icon(self.root, self.resources, self.info)

    def test_identity_metadata_cannot_be_overwritten(self):
        def invalid(args, **kwargs):
            self.compiler(args, **kwargs)
            Path(args[args.index("--output-partial-info-plist") + 1]).write_bytes(
                plistlib.dumps({"CFBundleIconName": "iceon", "CFBundleIdentifier": "other"}))
        with patch.object(icons.subprocess, "run", side_effect=invalid):
            with self.assertRaisesRegex(ValueError, "Unexpected"):
                icons.compile_icon(self.root, self.resources, self.info)
        self.assertEqual(self.info["CFBundleIdentifier"], "ad.yan.iceon")

    def test_missing_registration_or_fallback_is_rejected(self):
        for info in ({}, {"CFBundleIconName": "iceon", "CFBundleIconFile": "../icon.icns"},
                     {"CFBundleIconName": "iceon", "CFBundleIconFile": "absent.icns"}):
            with self.subTest(info=info), self.assertRaises(ValueError):
                icons.verify_icon(self.resources, info)

    def test_both_packagers_compile_icons_before_signing(self):
        for filename in ("distribution.py", "native_release.py"):
            source = Path(__file__).with_name(filename).read_text()
            self.assertIn("from macos_icons import compile_icon, verify_icon", source)
            self.assertLess(source.index("compile_icon(root,"), source.index('run("codesign", "--force"'))

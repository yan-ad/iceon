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
        host = patch.object(icons.platform, "mac_ver", return_value=("26.2", ("", "", ""), "arm64"))
        host.start()
        self.addCleanup(host.stop)
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
        if args == ["xcrun", "--find", "actool"]:
            return subprocess.CompletedProcess(args, 0, b"/Applications/Xcode_26.3.app/Contents/Developer/usr/bin/actool\n")
        if args == ["xcrun", "xcodebuild", "-version"]:
            return subprocess.CompletedProcess(args, 0, b"Xcode 26.3\nBuild version 17C529\n")
        if args == ["xcrun", "actool", "--output-format=xml1", "--version"]:
            return subprocess.CompletedProcess(args, 0, plistlib.dumps({
                "com.apple.actool.version": {"bundle-version": "24127"}}))
        self.assertEqual(args[:2], ["xcrun", "actool"])
        output = Path(args[args.index("--compile") + 1])
        self.assertEqual(args, [
            "xcrun", "actool", str(self.source), "--compile", str(output),
            "--output-format", "human-readable-text", "--notices", "--warnings", "--errors",
            "--lightweight-asset-runtime-mode", "enabled",
            "--app-icon", "iceon", "--platform", "macosx",
            "--include-all-app-icons", "--enable-on-demand-resources", "NO",
            "--target-device", "mac", "--minimum-deployment-target", "11.0",
            "--output-partial-info-plist", str(output / "icon-info.plist"),
        ])
        self.assertEqual(kwargs, {"check": True, "timeout": 300})
        (output / "Assets.car").write_bytes(b"compiled-appearance-asset")
        partial = Path(args[args.index("--output-partial-info-plist") + 1])
        partial.write_bytes(plistlib.dumps({"CFBundleIconName": "iceon", "CFBundleIconFile": "iceon.icns"}))

    def test_compiler_metadata_and_catalog_are_packaged_without_editing_source(self):
        with patch.object(icons.subprocess, "run", side_effect=self.compiler):
            icons.compile_icon(self.root, self.resources, self.info)
        icons.verify_icon(self.resources, self.info)
        self.assertEqual(self.info["CFBundleIdentifier"], "ad.yan.iceon")

    def test_unsupported_or_unknown_toolchain_is_rejected_before_compilation(self):
        cases = [
            (b"Xcode 16.4\nBuild version 16F6\n", "24127", "Xcode 26"),
            (b"unrecognized version", "24127", "Xcode 26"),
            (b"Xcode 26.0\nBuild version 17A324\n", "24000", "24127"),
            (b"Xcode 26.0\nBuild version 17A324\n", "unknown", "Cannot determine"),
            (b"Xcode 26.0\nBuild version 17A324\n", None, "Cannot determine"),
        ]
        for xcode, version, message in cases:
            def tool(args, **kwargs):
                self.assertNotIn("--compile", args)
                if "xcodebuild" in args:
                    return subprocess.CompletedProcess(args, 0, xcode)
                if "--version" in args:
                    metadata = {} if version is None else {"bundle-version": version}
                    return subprocess.CompletedProcess(args, 0, plistlib.dumps({"com.apple.actool.version": metadata}))
                return self.compiler(args, **kwargs)
            with self.subTest(version=version, xcode=xcode), patch.object(icons.subprocess, "run", side_effect=tool):
                with self.assertRaisesRegex(ValueError, message):
                    icons.compile_icon(self.root, self.resources, self.info)
            self.assertNotIn("CFBundleIconName", self.info)
            self.assertFalse((self.resources / "Assets.car").exists())
            self.assertEqual(self.document.read_bytes(), self.original)

    def test_incompatible_or_unknown_host_is_rejected_before_starting_tools(self):
        for host in ("15.7.9", "14.8", "", "unknown", "26.bad"):
            with self.subTest(host=host), patch.object(icons.platform, "mac_ver", return_value=(host, (), "")), \
                    patch.object(icons.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "macOS 26\\+ host"):
                    icons.compile_icon(self.root, self.resources, self.info)
                run.assert_not_called()
            self.assertNotIn("CFBundleIconName", self.info)
            self.assertFalse((self.resources / "Assets.car").exists())
            self.assertEqual(self.document.read_bytes(), self.original)

    def test_supported_hosts_keep_bundled_runtime_and_legacy_deployment_target(self):
        for host, machine in (("26.0", "arm64"), ("26.6.1", "x86_64"), ("27.0", "arm64")):
            with self.subTest(host=host, machine=machine), \
                    patch.object(icons.platform, "mac_ver", return_value=(host, (), machine)), \
                    patch.object(icons.subprocess, "run", side_effect=self.compiler):
                icons.compile_icon(self.root, self.resources, self.info)
                icons.verify_icon(self.resources, self.info)

    def test_zero_exit_with_empty_metadata_is_not_success(self):
        def empty(args, **kwargs):
            result = self.compiler(args, **kwargs)
            if "--compile" not in args:
                return result
            Path(args[args.index("--output-partial-info-plist") + 1]).write_bytes(plistlib.dumps({}))
            (Path(args[args.index("--compile") + 1]) / "Assets.car").unlink()
        with patch.object(icons.subprocess, "run", side_effect=empty):
            with self.assertRaisesRegex(ValueError, "primary Icon Composer"):
                icons.compile_icon(self.root, self.resources, self.info)
        self.assertNotIn("CFBundleIconName", self.info)

    def test_mac_workflows_select_supported_xcode_before_building(self):
        workflows = Path(__file__).parents[1] / "workflows"
        for filename, build_step in (("release.yml", "Build native app"), ("distribution.yml", "Check target")):
            source = (workflows / filename).read_text()
            selection = source.index("- name: Select Xcode 26 for Icon Composer")
            self.assertLess(selection, source.index("- name: " + build_step))
            self.assertIn('DEVELOPER_DIR=/Applications/Xcode_26.3.app/Contents/Developer', source)
            self.assertIn('if: runner.os == \'macOS\'', source[selection:selection + 600])
            self.assertIn('macos_icons.py --check-toolchain', source[selection:selection + 600])
            self.assertIn('- os: macos-26\n            target: aarch64-apple-darwin', source)
            self.assertNotIn('- os: macos-15\n', source)
            self.assertNotIn('- os: macos-15-intel\n', source)
        distribution = (workflows / "distribution.yml").read_text()
        self.assertIn('- os: macos-26-intel\n            target: x86_64-apple-darwin', distribution)
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
        def failed(args, **kwargs):
            if "--compile" in args:
                raise subprocess.CalledProcessError(1, "actool")
            return self.compiler(args, **kwargs)
        with patch.object(icons.subprocess, "run", side_effect=failed):
            with self.assertRaises(subprocess.CalledProcessError):
                icons.compile_icon(self.root, self.resources, self.info)

    def test_missing_compiled_catalog_is_rejected(self):
        def missing(args, **kwargs):
            result = self.compiler(args, **kwargs)
            if "--compile" not in args:
                return result
            (Path(args[args.index("--compile") + 1]) / "Assets.car").unlink()
        with patch.object(icons.subprocess, "run", side_effect=missing):
            with self.assertRaisesRegex(ValueError, "Assets.car"):
                icons.compile_icon(self.root, self.resources, self.info)

    def test_identity_metadata_cannot_be_overwritten(self):
        def invalid(args, **kwargs):
            result = self.compiler(args, **kwargs)
            if "--compile" not in args:
                return result
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

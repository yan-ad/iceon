"""Offline distribution tests: fixture binaries, bounded archives, mocked OS tools.

`python3 -m unittest discover -s .github/scripts -p test_distribution.py -v`
No real credentials, network, user configuration or application execution.
"""
import io
import os
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import distribution as d


def binary(triple):
    target = d.TARGETS[triple]
    data = bytearray(256)
    if target.system == "Darwin":
        data[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", data, 4, target.machine)
        struct.pack_into("<I", data, 12, 2)
    elif target.system == "Linux":
        data[:7] = b"\x7fELF\x02\x01\x01"
        struct.pack_into("<HHI", data, 16, 3, target.machine, 1)
    else:
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 64)
        data[64:68] = b"PE\0\0"
        struct.pack_into("<HH", data, 68, target.machine, 1)
        struct.pack_into("<HH", data, 84, 96 if target.arch == "i686" else 112, 2)
        struct.pack_into("<H", data, 88, 0x10B if target.arch == "i686" else 0x20B)
    return bytes(data)


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.5"\n')
        (self.root / "LICENSE").write_text("Fixture license\n")
        (self.root / "assets/icons").mkdir(parents=True)
        for name in ("icon.ico", "icon.icns", "icon.png"):
            (self.root / "assets/icons" / name).write_bytes(b"fixture")
        self.output = self.root / "output"

    def payload(self, triple):
        path = d.executable(self.root, triple)
        path.parent.mkdir(parents=True)
        path.write_bytes(binary(triple))
        return path

    def test_target_matrix_and_exact_binary_architecture(self):
        self.assertEqual(len(d.TARGETS), 6)
        for triple, target in d.TARGETS.items():
            with self.subTest(triple=triple):
                d.verify_binary(self.payload(triple), target)
                for other in d.TARGETS:
                    if other != triple:
                        with self.assertRaises(ValueError):
                            d.binary_header(binary(other), target)
                for bad in (b"", b"MZ", b"\x7fELF", binary(triple)[:25]):
                    with self.assertRaises(ValueError):
                        d.binary_header(bad, target)

    def test_reject_universal_mach_dll_wrong_class_and_offsets(self):
        mac = "aarch64-apple-darwin"
        windows = "aarch64-pc-windows-msvc"
        linux = "x86_64-unknown-linux-gnu"
        changes = [(mac, 0, b"\xca\xfe\xba\xbe"),
                   (mac, 12, struct.pack("<I", 6)),
                   (windows, 60, struct.pack("<I", 0xFFFFFFFF)),
                   (windows, 60, struct.pack("<I", 0)),
                   (windows, 86, struct.pack("<H", 0x2002)),
                   (windows, 84, struct.pack("<H", 65535)),
                   (windows, 88, struct.pack("<H", 0x10B)),
                   (linux, 4, b"\x01"), (linux, 5, b"\x02"),
                   (linux, 16, struct.pack("<H", 1))]
        for triple, offset, replacement in changes:
            with self.subTest(triple=triple, offset=offset):
                data = bytearray(binary(triple))
                data[offset:offset + len(replacement)] = replacement
                with self.assertRaises(ValueError):
                    d.binary_header(bytes(data), d.TARGETS[triple])

    def test_validate_version_rejects_hostile_and_mismatch(self):
        d.validate_version(self.root, "0.1.5")
        for value in ("../0.1.5", "0.1.5\n", "0.1.5-rc.1", "0.1.5+build", "01.1.5",
                      "0.1.5/evil", "65536.1.5", "0.1.5;echo", "0.1.6", "", "1.2"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                d.validate_version(self.root, value)

    def test_symlink_payload_parent_output_and_oversized_binary_fail(self):
        triple = "x86_64-unknown-linux-gnu"
        path = self.payload(triple)
        real = path.with_name("actual")
        path.rename(real)
        path.symlink_to(real)
        with self.assertRaises(ValueError):
            d.verify_binary(path, d.TARGETS[triple])
        with self.assertRaises(ValueError):
            d.no_symlinks(path)
        self.output.symlink_to(path.parent, target_is_directory=True)
        with self.assertRaises(ValueError):
            d.no_symlinks(self.output / "new-directory")
        with patch.object(d, "MAX_BINARY", 100), self.assertRaises(ValueError):
            # regular() has an explicit default bound, verify through explicit bound.
            d.regular(real, d.MAX_BINARY)
        real.write_bytes(b"")
        with self.assertRaises(ValueError):
            d.verify_binary(real, d.TARGETS[triple])

    def test_build_uses_locked_explicit_target_and_checks_result(self):
        triple = "i686-pc-windows-msvc"
        self.payload(triple)
        with patch.object(d, "run") as run:
            d.build(self.root, triple)
        run.assert_called_once_with("cargo", "build", "--locked", "--release", "-p",
                                    "iceon-app", "--target", triple, cwd=self.root)

    def test_package_rejects_wrong_host_nonempty_output_and_bad_arch_before_tools(self):
        triple = "x86_64-unknown-linux-gnu"
        exe = self.payload(triple)
        with patch.object(d.platform, "system", return_value="Windows"), self.assertRaises(ValueError):
            d.package(self.root, triple, "0.1.5", self.output)
        self.output.mkdir()
        (self.output / "user-file").write_text("keep")
        with patch.object(d.platform, "system", return_value="Linux"), patch.object(d, "run") as run:
            with self.assertRaisesRegex(ValueError, "empty"):
                d.package(self.root, triple, "0.1.5", self.output)
            exe.write_bytes(binary("aarch64-unknown-linux-gnu"))
            with self.assertRaisesRegex(ValueError, "architecture"):
                d.package(self.root, triple, "0.1.5", self.output)
            run.assert_not_called()
        self.assertEqual((self.output / "user-file").read_text(), "keep")

    def tar(self, entries):
        path = self.root / "test.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, payload, kind in entries:
                entry = tarfile.TarInfo(name)
                entry.mode = 0o755
                entry.type = kind
                entry.size = len(payload)
                if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                    entry.linkname = "/etc/passwd"
                    entry.size = 0
                archive.addfile(entry, io.BytesIO(payload) if entry.isfile() else None)
        return path

    def test_tar_safe_payload_and_hostile_paths_duplicates_links(self):
        triple = "x86_64-unknown-linux-gnu"
        target = d.TARGETS[triple]
        good = ("Iceon/bin/iceon", binary(triple), tarfile.REGTYPE)
        d.verify_archive(self.tar([good]), target, good[0])
        for name in ("../escape", "/absolute", "Iceon/../escape", "Iceon\\escape", "C:/evil", "./Iceon", "Iceon//evil"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                d.verify_archive(self.tar([good, (name, b"x", tarfile.REGTYPE)]), target, good[0])
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                d.verify_archive(self.tar([good, ("Iceon/link", b"", kind)]), target, good[0])
        with self.assertRaisesRegex(ValueError, "Duplicate"):
            d.verify_archive(self.tar([good, good]), target, good[0])
        with self.assertRaisesRegex(ValueError, "missing"):
            d.verify_archive(self.tar([]), target, good[0])

    def test_tar_bounds_and_no_extraction(self):
        triple = "aarch64-unknown-linux-gnu"
        good = ("Iceon/bin/iceon", binary(triple), tarfile.REGTYPE)
        path = self.tar([good, ("Iceon/LICENSE", b"license", tarfile.REGTYPE)])
        with patch.object(d, "MAX_ENTRIES", 1), self.assertRaisesRegex(ValueError, "entries"):
            d.verify_archive(path, d.TARGETS[triple], good[0])
        with patch.object(d, "MAX_ARCHIVE_BYTES", 100), self.assertRaises(ValueError):
            d.verify_archive(path, d.TARGETS[triple], good[0])
        with patch.object(d, "MAX_BINARY", 100), self.assertRaisesRegex(ValueError, "size limit"):
            d.verify_archive(path, d.TARGETS[triple], good[0])
        self.assertFalse((self.root / "Iceon").exists())

    def test_zip_rejects_links_paths_missing_and_wrong_arch(self):
        path = self.root / "test.zip"
        triple = "aarch64-pc-windows-msvc"
        for name, payload, mode in (("../escape", binary(triple), 0o100755),
                                    ("Iceon/iceon.exe", binary(triple), 0o120777),
                                    ("Iceon/iceon.exe", binary("i686-pc-windows-msvc"), 0o100755),
                                    ("other.exe", binary(triple), 0o100755)):
            with self.subTest(name=name, mode=mode):
                with zipfile.ZipFile(path, "w") as archive:
                    entry = zipfile.ZipInfo(name)
                    entry.external_attr = mode << 16
                    archive.writestr(entry, payload)
                with self.assertRaises(ValueError):
                    d.verify_archive(path, d.TARGETS[triple], "Iceon/iceon.exe", zipped=True)

    def test_zip_directory_bound_checked_before_zipfile_allocation(self):
        path = self.root / "test.zip"
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("Iceon/iceon.exe", binary("i686-pc-windows-msvc"))
        data = bytearray(path.read_bytes())
        end = data.rfind(b"PK\x05\x06")
        struct.pack_into("<I", data, end + 12, d.MAX_HEADER + 1)
        path.write_bytes(data)
        with patch.object(d.zipfile, "ZipFile") as reader, self.assertRaisesRegex(ValueError, "directory limits"):
            d.verify_archive(path, d.TARGETS["i686-pc-windows-msvc"], "Iceon/iceon.exe", zipped=True)
        reader.assert_not_called()

    def test_linux_requires_matching_native_debian_architecture(self):
        triple = "aarch64-unknown-linux-gnu"
        exe = self.payload(triple)
        stage = self.root / "stage"
        stage.mkdir()
        with patch.object(d, "run", return_value="amd64") as run, self.assertRaisesRegex(ValueError, "Debian architecture"):
            d.package_linux(self.root, stage, self.output, exe, d.TARGETS[triple], "0.1.5", "fixture")
        run.assert_called_once_with("dpkg", "--print-architecture")

    def test_windows_target_selects_separate_installer_x86_bootstrap(self):
        for triple in ("i686-pc-windows-msvc", "aarch64-pc-windows-msvc"):
            with self.subTest(triple=triple):
                exe = self.payload(triple)
                out = self.root / triple
                out.mkdir()
                target = d.TARGETS[triple]
                stem = f"Iceon_0.1.5_{target.arch}"
                def compile_nsis(*args):
                    output = next(a.split("DOUTPUT=", 1)[1] for a in args if "DOUTPUT=" in a)
                    Path(output).write_bytes(binary("i686-pc-windows-msvc"))
                with patch.object(d, "run", side_effect=compile_nsis) as run:
                    d.package_windows(self.root, out, exe, target, "0.1.5", stem)
                expected = "iceon-arm64.nsi" if target.arch == "aarch64" else "iceon.nsi"
                self.assertEqual(Path(run.call_args.args[-1]).name, expected)
                d.verify_archive(out / (stem + ".zip"), target, "Iceon/iceon.exe", zipped=True)

    def test_linux_uses_real_shlibdeps_output_and_verifies_deb_fields(self):
        triple = "aarch64-unknown-linux-gnu"
        exe = self.payload(triple)
        metadata = self.root / "packaging/linux"
        metadata.mkdir(parents=True)
        for name in ("ad.yan.iceon.desktop", "ad.yan.iceon.metainfo.xml"):
            (metadata / name).write_text("fixture metadata")
        stage = self.root / "stage"
        stage.mkdir()
        out = self.root / "artifacts"
        out.mkdir()
        deps = "libc6 (>= 2.38), libxkbcommon0 (>= 1.0.0)"
        fields = {"Package": "iceon", "Version": "0.1.5", "Architecture": "arm64", "Depends": deps}
        def tools(*args, **kwargs):
            if args[:2] == ("dpkg", "--print-architecture"):
                return "arm64"
            if args[0] == "dpkg-shlibdeps":
                self.assertIn("-O", args)
                self.assertTrue(any(str(a).startswith("-e") for a in args))
                self.assertEqual(kwargs["cwd"], stage)
                return "shlibs:Depends=" + deps
            if args[:2] == ("dpkg-deb", "--build"):
                self.assertIn("--root-owner-group", args)
                Path(args[-1]).write_bytes(b"fixture deb")
                return ""
            return fields[args[-1]]
        with patch.object(d, "run", side_effect=tools):
            d.package_linux(self.root, stage, out, exe, d.TARGETS[triple], "0.1.5", "Iceon_0.1.5_aarch64")
        control = (stage / "deb/DEBIAN/control").read_text()
        self.assertIn("Depends: " + deps, control)
        self.assertIn("Architecture: arm64", control)
        self.assertTrue((out / "iceon_0.1.5_arm64.deb").is_file())

    def test_shlibdeps_fails_closed_for_absent_or_hostile_results(self):
        for result in ("", "shlibs:Depends=", "noise", "shlibs:Depends=libc6\nInjected: yes", "shlibs:Depends=bad\x00"):
            with self.subTest(result=result):
                stage = self.root / str(len(list(self.root.iterdir())))
                stage.mkdir()
                with patch.object(d, "run", return_value=result), self.assertRaises(ValueError):
                    d.shlib_dependencies(stage, stage / "binary")

    def test_failed_preparation_leaves_no_partial_artifacts(self):
        triple = "i686-pc-windows-msvc"
        self.payload(triple)
        with patch.object(d.platform, "system", return_value="Windows"), patch.object(d, "package_windows", side_effect=ValueError("fixture failure")):
            with self.assertRaisesRegex(ValueError, "fixture failure"):
                d.package(self.root, triple, "0.1.5", self.output)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_arm_installer_contract_does_not_reuse_legacy_identity(self):
        script = (d.ROOT / ".github/installer/iceon-arm64.nsi").read_text()
        for text in ('!include "x64.nsh"', '!include "WinVer.nsh"', '${IsNativeARM64}',
                     '${AtLeastWin11}', 'SetRegView 64', '$LOCALAPPDATA\\Iceon ARM64',
                     'Uninstall\\IceonARM64', '$SMPROGRAMS\\Iceon ARM64.lnk'):
            self.assertIn(text, script)
        self.assertEqual(script.count('${IsNativeARM64}'), 2)
        self.assertEqual(script.count('${AtLeastWin11}'), 2)
        self.assertNotIn('Software\\Yanuar\\Iceon', script)
        self.assertNotIn('SetRegView 32', script)


if __name__ == "__main__":
    unittest.main()

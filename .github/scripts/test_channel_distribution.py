#!/usr/bin/env python3
"""Offline channel preparation tests; no user configuration or real credentials."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import tarfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

import channel_distribution as channel


class ChannelTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.addCleanup(self.tmp.cleanup)
        self.version = "0.1.5"

    def artifacts(self):
        sums = {}
        for arch in channel.ARCHES.values():
            name = f"Iceon_{self.version}_{arch}.dmg"
            (self.root / name).write_bytes(("synthetic-test-only-" + arch).encode())
            sums[name] = channel.digest(self.root / name)
        return sums

    def test_real_byte_hashes_and_arch_mapping(self):
        sums = self.artifacts()
        text = channel.cask(self.version, sums, self.root)
        self.assertIn('arch arm: "aarch64", intel: "x86_64"', text)
        self.assertIn('on_arm do\n    sha256 "' + sums[f"Iceon_{self.version}_aarch64.dmg"], text)
        self.assertIn('on_intel do\n    sha256 "' + sums[f"Iceon_{self.version}_x86_64.dmg"], text)
        self.assertIn("https://github.com/yan-ad/iceon/releases/download/#{version}", text)
        self.assertNotIn("no_check", text)

    def test_missing_arch_and_mismatch_fail(self):
        sums = self.artifacts()
        missing = dict(sums)
        del missing[f"Iceon_{self.version}_x86_64.dmg"]
        with self.assertRaises(ValueError):
            channel.cask(self.version, missing)
        sums[f"Iceon_{self.version}_aarch64.dmg"] = hashlib.sha256(b"wrong-test-bytes").hexdigest()
        with self.assertRaises(ValueError):
            channel.cask(self.version, sums, self.root)

    def test_sha_and_version_input(self):
        for invalid in ["no_check", "g" * 64, "a" * 63, "a" * 65, "a" * 64 + "\n"]:
            with self.assertRaises(ValueError):
                channel.sha(invalid)
        for version in ["latest", "0.1.5/../x", "0.1.5\"", "9.9.9"]:
            with self.assertRaises(ValueError):
                channel.cask(version, {})

    def test_checksum_syntax(self):
        path = self.root / "SHA256SUMS"
        sums = self.artifacts()
        path.write_text("".join(value.upper() + " *" + name + "\n" for name, value in sums.items()))
        self.assertEqual(channel.checksums(path), sums)
        line = next(iter(sums.values())) + "  file.dmg\n"
        for text in [line + line, "", "not-a-checksum", next(iter(sums.values())) + "  ../file.dmg"]:
            path.write_text(text)
            with self.assertRaises(ValueError):
                channel.checksums(path)

    def test_no_output_without_inputs_cli(self):
        output = self.root / "tap"
        result = subprocess.run(["python3", str(channel.ROOT / ".github/scripts/channel_distribution.py"),
                                 "homebrew", "--version", self.version, "--output", str(output)],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(output.exists())

    def test_vendoring_is_locked_then_build_metadata_frozen(self):
        config = '[source.vendored-sources]\ndirectory = "vendor"\n'
        for network in [False, True]:
            stage = self.root / ("network" if network else "offline")
            stage.mkdir()
            with patch.object(channel, "copy_source"), patch.object(channel.subprocess, "check_output",
                    return_value=config) as vendor, patch.object(channel.subprocess, "run") as metadata:
                channel.prepare_flatpak(stage, network)
            command = vendor.call_args.args[0]
            self.assertIn("--locked", command)
            self.assertEqual("--offline" in command, not network)
            self.assertEqual(command[-1], "vendor")
            self.assertEqual(metadata.call_args.args[0][:4], ["cargo", "metadata", "--frozen", "--offline"])
            self.assertEqual((stage / "source/.cargo/config.toml").read_text(), config)
            self.assertEqual(json.loads((stage / "ad.yan.iceon.json").read_text()), channel.manifest())

    def test_export_is_atomic_and_no_overwrite(self):
        output = self.root / "out"
        def fail(stage):
            (stage / "partial").write_text("incomplete")
            raise ValueError("test failure")
        with self.assertRaises(ValueError):
            channel.export(output, fail)
        self.assertFalse(output.exists())
        channel.export(output, lambda stage: (stage / "ok").write_text("ok"))
        with self.assertRaises(ValueError):
            channel.export(output, fail)
        self.assertEqual((output / "ok").read_text(), "ok")

    def test_source_allowlist(self):
        for name in [".env", ".git/config", "target/release/iceon", "crates/a/src/.env",
                     "crates/a/src/credentials/file.json", "crates/a/src/key.pem", "../Cargo.toml",
                     "/Cargo.toml", "assets/icons/unexpected.png", ".cargo/config.toml"]:
            self.assertFalse(channel.allowed_source(name), name)
        for name in ["Cargo.lock", "crates/iceon-app/src/main.rs", "themes/dark.json",
                     "assets/icons/32x32.png", "assets/icons/icon.icns", "assets/icons/icon.ico",
                     "crates/iceon-ui/assets/fonts/font.ttf", "packaging/linux/ad.yan.iceon.desktop"]:
            self.assertTrue(channel.allowed_source(name), name)

    def test_source_copy_ignores_untracked_and_rejects_symlink(self):
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "Cargo.lock").write_text("fixture")
        (self.root / ".env").write_text("synthetic-test-only")
        metadata = self.root / "packaging/linux"
        metadata.mkdir(parents=True)
        for name in ["ad.yan.iceon.desktop", "ad.yan.iceon.metainfo.xml"]:
            (metadata / name).write_text("fixture")
        subprocess.run(["git", "add", "Cargo.lock", ".env"], cwd=self.root, check=True)
        target = self.root / "stage"
        channel.copy_source(self.root, target)
        self.assertTrue((target / "Cargo.lock").exists())
        self.assertFalse((target / ".env").exists())
        (self.root / "Cargo.lock").unlink()
        (self.root / "Cargo.lock").symlink_to(metadata / "ad.yan.iceon.desktop")
        with self.assertRaises(ValueError):
            channel.copy_source(self.root, self.root / "stage2")

    def test_vendoring_fails_without_export(self):
        output = self.root / "flatpak"
        with patch.object(channel, "copy_source"), patch.object(channel.subprocess, "check_output",
                side_effect=subprocess.CalledProcessError(1, ["cargo", "vendor"])):
            with self.assertRaises(subprocess.CalledProcessError):
                channel.export(output, channel.prepare_flatpak)
        self.assertFalse(output.exists())

    def test_local_manifest_and_metadata(self):
        manifest = channel.manifest()
        self.assertEqual(manifest["runtime-version"], "25.08")
        self.assertEqual(manifest["sdk-extensions"], ["org.freedesktop.Sdk.Extension.rust-stable"])
        self.assertEqual(manifest["modules"][0]["sources"], [{"type": "dir", "path": "source"}])
        self.assertIn("cargo build --frozen --offline --release -p iceon-app", manifest["modules"][0]["build-commands"])
        self.assertEqual(set(manifest["finish-args"]), {
            "--share=network", "--share=ipc", "--device=dri", "--socket=wayland",
            "--socket=fallback-x11", "--talk-name=org.freedesktop.secrets"})
        desktop = (channel.ROOT / "packaging/linux/ad.yan.iceon.desktop").read_text()
        self.assertNotIn("x-scheme-handler", desktop)
        self.assertIn("Exec=iceon\n", desktop)
        info = ET.parse(channel.ROOT / "packaging/linux/ad.yan.iceon.metainfo.xml").getroot()
        self.assertEqual(info.findtext("id"), manifest["app-id"])
        self.assertEqual(info.findtext("project_license"), "MIT")

    def test_remote_requires_hash_revision_and_safe_generated_sources(self):
        archive = self.root / "source.tar.gz"
        revision = "abcdef0123" * 4
        with tarfile.open(archive, "w:gz") as contents:
            lock = b'version = 4\npackage = []\n'
            member = tarfile.TarInfo(f"iceon-{revision}/Cargo.lock")
            member.size = len(lock)
            contents.addfile(member, io.BytesIO(lock))
        sources_file = self.root / "cargo-sources.json"
        sources = [{"type": "inline", "dest": "cargo", "dest-filename": "config", "contents":
                    '[source.vendored-sources]\ndirectory = "cargo/vendor"\n[source.crates-io]\nreplace-with = "vendored-sources"\n'}]
        sources_file.write_text(json.dumps(sources))
        stage = self.root / "stage"
        stage.mkdir()
        revision = "abcdef0123" * 4
        checksum = channel.digest(archive)
        channel.prepare_remote(stage, revision, archive, checksum, sources_file)
        data = json.loads((stage / "ad.yan.iceon.json").read_text())
        source = data["modules"][0]["sources"][0]
        self.assertEqual(source["sha256"], checksum)
        self.assertEqual(source["url"], f"https://github.com/yan-ad/iceon/archive/{revision}.tar.gz")
        for rev, value in [("main", checksum), (revision, "bad")]:
            with self.assertRaises(ValueError):
                channel.prepare_remote(stage, rev, archive, value, sources_file)
        for item in [{"type": "shell", "commands": ["false"]},
                     {"type": "inline", "dest": "../escape", "dest-filename": "bad", "contents": ""},
                     {"type": "archive", "dest": "cargo/vendor/foo", "url": "https://evil.test/x", "sha256": checksum}]:
            sources_file.write_text(json.dumps([item]))
            with self.assertRaises(ValueError):
                channel.prepare_remote(stage, revision, archive, checksum, sources_file)

        sources_file.write_text(json.dumps(sources + [{
            "type": "archive", "dest": "cargo/vendor/not-in-lock-1.0.0",
            "url": "https://static.crates.io/crates/not-in-lock/not-in-lock-1.0.0.crate",
            "sha256": checksum}]))
        with self.assertRaises(ValueError):
            channel.prepare_remote(stage, revision, archive, checksum, sources_file)

        for name, kind in [(f"iceon-{revision}/../escape", tarfile.REGTYPE),
                           (f"iceon-{revision}/Cargo.lock", tarfile.SYMTYPE)]:
            with tarfile.open(archive, "w:gz") as contents:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "/outside"
                contents.addfile(member)
            with self.assertRaises(ValueError):
                channel.prepare_remote(stage, revision, archive, channel.digest(archive), sources_file)


if __name__ == "__main__":
    unittest.main()

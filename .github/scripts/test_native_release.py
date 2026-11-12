"""Stdlib regression tests; no network, publication, Rust build or OS tools."""
import base64
import copy
import io
import json
import os
from pathlib import Path
import plistlib
import struct
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from unittest.mock import patch

import native_release as n


def mach():
    return b"\xcf\xfa\xed\xfe" + struct.pack("<I", 0x100000C) + b"\0" * 24


def pe():
    data = bytearray(128)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 60, 64)
    data[64:68] = b"PE\0\0"
    struct.pack_into("<H", data, 68, 0x14C)
    struct.pack_into("<H", data, 88, 0x10B)
    return bytes(data)


class NativeReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / "dist"
        self.out.mkdir()
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.3"\n')
        lock = []
        for name in n.PACKAGES:
            directory = self.root / "crates" / name
            directory.mkdir(parents=True)
            (directory / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion.workspace = true\n')
            lock.append(f'[[package]]\nname = "{name}"\nversion = "0.1.3"\n')
        (self.root / "Cargo.lock").write_text("\n".join(lock))
        (self.root / "CHANGELOG.md").write_text("# Changes\n\n## 0.1.3 (pending CI)\n\nNative release.\n\n## 0.1.1\nOld\n")
        self.sha = "a" * 40
        self.tag = "b" * 40
        self.verifier = patch.object(n, "verify_updates")
        self.verifier.start()
        self.addCleanup(self.verifier.stop)
        self.env = {"GITHUB_REPOSITORY": n.REPOSITORY, "RELEASE_TAG": n.VERSION, "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/tags/0.1.3", "GITHUB_SHA": self.sha}

    def assets(self, extra=None):
        info = dict(CFBundleIdentifier="ad.yan.iceon", CFBundleExecutable="iceon", CFBundleVersion=n.VERSION, CFBundleShortVersionString=n.VERSION)
        entries = {"Iceon.app/Contents/Info.plist": plistlib.dumps(info), "Iceon.app/Contents/MacOS/iceon": mach(), "Iceon.app/Contents/_CodeSignature/CodeResources": b"signature"}
        if extra:
            entries.update(extra)
        with tarfile.open(self.out / n.MAC, "w:gz") as archive:
            for name, data in entries.items():
                member = tarfile.TarInfo(name)
                member.size = len(data)
                member.mode = 0o755
                archive.addfile(member, io.BytesIO(data))
        (self.out / n.WIN).write_bytes(pe())
        with zipfile.ZipFile(self.out / n.PORTABLE, "w") as archive:
            archive.writestr("Iceon/iceon.exe", pe())
        for name in (n.MAC, n.WIN):
            document = "untrusted comment: fixture\npacket\ntrusted comment: timestamp:1\tfile:" + name + "\tversion:" + n.VERSION + "\nglobal\n"
            (self.out / (name + ".sig")).write_text(base64.b64encode(document.encode()).decode())
        n.updater_manifest(self.out, create=True)
        (self.out / n.DMG).write_bytes(b"koly" + b"\0" * 508)

    def git(self, *args):
        if "ls-remote" in args:
            return f"{self.tag}\trefs/tags/0.1.3\n{self.sha}\trefs/tags/0.1.3^{{}}"
        return self.sha

    def test_portable_upgrade_keeps_012_archive_layout(self):
        self.assets()
        n.verify_portable(self.out / n.PORTABLE)
        for name, data in [("../iceon.exe", pe()), ("Iceon/iceon.exe", mach())]:
            with zipfile.ZipFile(self.out / n.PORTABLE, "w") as archive:
                archive.writestr(name, data)
            with self.assertRaises(ValueError):
                n.verify_portable(self.out / n.PORTABLE)

    def test_versions_valid_inherited_and_explicit(self):
        n.versions(self.root)
        path = self.root / "crates/iceon-app/Cargo.toml"
        path.write_text('[package]\nname = "iceon-app"\nversion = "0.1.3"\n')
        n.versions(self.root)

    def test_versions_fail_closed(self):
        for filename in ("Cargo.toml", "Cargo.lock", "crates/iceon-app/Cargo.toml"):
            path = self.root / filename
            original = path.read_text()
            path.write_text(original.replace('"0.1.3"', '"0.1.8"') if filename != "crates/iceon-app/Cargo.toml" else original.replace("version.workspace = true", 'version = "0.1.8"'))
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                n.versions(self.root)
            path.write_text(original)

    def test_lock_missing_duplicate_or_registry_package(self):
        path = self.root / "Cargo.lock"
        original = path.read_text()
        for text in (original.replace('name = "iceon-app"', 'name = "other"'), original + '\n[[package]]\nname = "iceon-app"\nversion = "0.1.3"\n', original.replace('name = "iceon-app"', 'name = "iceon-app"\nsource = "registry"')):
            path.write_text(text)
            with self.assertRaises(ValueError):
                n.versions(self.root)

    def test_changelog_section_only(self):
        self.assertEqual(n.changelog(self.root), "Native release.")
        for text in ("## 0.1.1\nOld", "## v0.1.3\nNo", "## 0.1.3\n\n", "## 0.1.3\nOne\n## 0.1.3\nTwo"):
            (self.root / "CHANGELOG.md").write_text(text)
            with self.assertRaises(ValueError):
                n.changelog(self.root)

    def test_guard_annotated_and_manual_same_commit(self):
        with patch.dict(os.environ, self.env, clear=True), patch.object(n, "run", side_effect=self.git):
            self.assertEqual(n.guard(self.root), self.tag)
            os.environ["GITHUB_EVENT_NAME"] = "workflow_dispatch"
            os.environ["GITHUB_REF"] = "refs/heads/super"
            self.assertEqual(n.guard(self.root, self.tag), self.tag)

    def test_guard_rejects_bad_context(self):
        for key, value in (("GITHUB_REPOSITORY", "fork/iceon"), ("RELEASE_TAG", "v0.1.3"), ("RELEASE_TAG", "0.1.4"), ("GITHUB_EVENT_NAME", "pull_request"), ("GITHUB_REF", "refs/heads/0.1.3"), ("GITHUB_SHA", "invalid")):
            with patch.dict(os.environ, dict(self.env, **{key: value}), clear=True), patch.object(n, "run", side_effect=self.git), self.subTest(key=key, value=value), self.assertRaises(ValueError):
                n.guard(self.root)

    def test_guard_rejects_local_commit_mismatch(self):
        with patch.dict(os.environ, self.env, clear=True), patch.object(n, "run", return_value="c" * 40), self.assertRaises(ValueError):
            n.guard(self.root)

    def test_remote_tag_missing_changed_or_different_commit(self):
        for remote in ("", f"{'c' * 40}\trefs/tags/0.1.3", f"{'c' * 40}\trefs/tags/0.1.3\n{self.sha}\trefs/tags/0.1.3^{{}}"):
            with patch.object(n, "run", return_value=remote), self.assertRaises(ValueError):
                n.remote_tag(self.root, self.sha, self.tag)
        with patch.object(n, "run", return_value=f"{self.sha}\trefs/tags/0.1.3"):
            self.assertEqual(n.remote_tag(self.root, self.sha), self.sha)

    def test_binary_headers(self):
        n.binary(mach(), "mac")
        n.binary(pe(), "windows")
        for data, platform in ((b"", "mac"), (pe(), "mac"), (mach(), "windows"), (b"MZ" + b"\xff" * 100, "windows"), (pe()[:70], "windows")):
            with self.assertRaises(ValueError):
                n.binary(data, platform)
        bad = bytearray(pe())
        struct.pack_into("<H", bad, 68, 0x8664)
        with self.assertRaises(ValueError):
            n.binary(bad, "windows")

    def test_checksum_roundtrip_and_tamper(self):
        self.assets()
        n.checksums(self.out, create=True)
        n.checksums(self.out)
        (self.out / n.WIN).write_bytes(b"tampered")
        with self.assertRaises(ValueError):
            n.checksums(self.out)

    def test_checksum_rejects_extra_missing_duplicate(self):
        self.assets()
        (self.out / "extra.json").write_text("{}")
        with self.assertRaises(ValueError):
            n.checksums(self.out, create=True)

        (self.out / "extra.json").unlink()
        n.checksums(self.out, create=True)
        text = (self.out / n.SUMS).read_text()
        for altered in (text + text.splitlines()[0] + "\n", text.splitlines()[0] + "\n", text.replace(n.WIN, "../bad.zip")):
            (self.out / n.SUMS).write_text(altered)
            with self.assertRaises(ValueError):
                n.checksums(self.out)

    def test_bundle_metadata_and_signature_validation(self):
        app = self.root / "Iceon.app"
        (app / "Contents/MacOS").mkdir(parents=True)
        exe = app / "Contents/MacOS/iceon"
        exe.write_bytes(mach())
        exe.chmod(0o755)
        metadata = dict(CFBundleIdentifier="ad.yan.iceon", CFBundleExecutable="iceon", CFBundleVersion=n.VERSION, CFBundleShortVersionString=n.VERSION, CFBundlePackageType="APPL")
        info = app / "Contents/Info.plist"
        info.write_bytes(plistlib.dumps(metadata))
        with patch.object(n, "run"), patch.object(n.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stderr="Signature=adhoc\n")):
            n.verify_bundle(app)
            exe.chmod(0o644)
            with self.assertRaises(ValueError):
                n.verify_bundle(app)
            exe.chmod(0o755)
            info.write_bytes(plistlib.dumps(dict(metadata, CFBundleIconFile="absent.icns")))
            with self.assertRaises(ValueError):
                n.verify_bundle(app)
            info.write_bytes(plistlib.dumps(dict(metadata, CFBundleVersion="0.1.8")))
            with self.assertRaises(ValueError):
                n.verify_bundle(app)
            info.write_bytes(plistlib.dumps(metadata))
        with patch.object(n, "run"), patch.object(n.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, stderr="Signature=Developer ID")), self.assertRaises(ValueError):
            n.verify_bundle(app)

    def test_fingerprint_rejects_symlink_empty_directory(self):
        (self.root / "empty").touch()
        (self.root / "link").symlink_to(self.root / "empty")
        for path in (self.root, self.root / "empty", self.root / "link"):
            with self.assertRaises(ValueError):
                n.fingerprint(path)

    def test_tar_rejects_traversal_and_missing_signature(self):
        self.assets({"Iceon.app/../../escape": b"bad"})
        with self.assertRaises(ValueError):
            n.verify_tar(self.out / n.MAC)
        with tarfile.open(self.out / n.MAC, "w:gz") as archive:
            member = tarfile.TarInfo("Iceon.app")
            member.type = tarfile.SYMTYPE
            member.linkname = "/tmp"
            archive.addfile(member)
        with self.assertRaises(ValueError):
            n.verify_tar(self.out / n.MAC)
        with tarfile.open(self.out / n.MAC, "w:gz"):
            pass
        with self.assertRaises(ValueError):
            n.verify_tar(self.out / n.MAC)

    def test_installer_rejects_non_windows_binary(self):
        (self.out / n.WIN).write_bytes(mach())
        with self.assertRaises(ValueError):
            n.verify_installer(self.out / n.WIN)

    def test_windows_package(self):
        exe = self.root / "target/i686-pc-windows-msvc/release/iceon.exe"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(pe())
        def compiler(*args):
            self.assertIn("DVERSION=" + n.VERSION, " ".join(args))
            self.assertTrue(args[-1].endswith("iceon.nsi"))
            (self.out / n.WIN).write_bytes(pe())
        with patch.object(n, "run", side_effect=compiler):
            n.package(self.root, self.out, "windows")
        n.verify_installer(self.out / n.WIN)
        with self.assertRaises(ValueError):
            n.package(self.root, self.out, "windows")

    def test_feed_exact_platforms_urls_and_signatures(self):
        self.assets()
        feed = self.out / n.FEED
        original = feed.read_text()
        for change in ("version", "url", "signature", "alias", "linux", "date"):
            manifest = json.loads(original)
            if change == "version":
                manifest["version"] = "0.1.4"
            elif change == "alias":
                del manifest["platforms"]["darwin-aarch64-app"]
            elif change == "linux":
                manifest["platforms"]["linux-x86_64"] = {}
            elif change == "date":
                manifest["pub_date"] = "2026-99-99T00:00:00Z"
            else:
                manifest["platforms"]["windows-i686"][change] = "bad"
            feed.write_text(json.dumps(manifest))
            with self.subTest(change=change), self.assertRaises(ValueError):
                n.updater_manifest(self.out)
        feed.write_text(original.replace('"version":', '"version":"0.1.3", "version":', 1))
        with self.assertRaises(ValueError):
            n.updater_manifest(self.out)
        feed.write_text(original)
        n.updater_manifest(self.out)
        with self.assertRaises(ValueError):
            n.updater_manifest(self.out, create=True)

    def test_signature_encoding_and_version(self):
        path = self.out / "fixture.sig"
        for document in ("bad", "untrusted comment: x\na\ntrusted comment: timestamp:1\nb", "untrusted comment: x\na\ntrusted comment: version:0.1.4\nb", "untrusted comment: x\na\ntrusted comment: version:0.1.3\tversion:0.1.3\nb"):
            path.write_text(base64.b64encode(document.encode()).decode())
            with self.assertRaises(ValueError):
                n.update_signature(path)
        path.write_text("invalid base64")
        with self.assertRaises(ValueError):
            n.update_signature(path)

    def test_crypto_verification_required_for_both_payloads(self):
        self.assets()
        self.verifier.stop()
        with patch.object(n, "run") as run:
            n.verify_updates(self.out)
            self.assertEqual(run.call_count, 2)
            self.assertTrue(all(c.args[:3] == ("cargo", "xtask", "verify-update") for c in run.call_args_list))
        with patch.object(n, "run", side_effect=RuntimeError("signature mismatch")), self.assertRaises(RuntimeError):
            n.updater_manifest(self.out)
        self.verifier.start()

    def test_signing_refuses_unexpected_files(self):
        self.assets()
        with patch.object(n, "run") as run, self.assertRaises(ValueError):
            n.sign_updates(self.out)
        run.assert_not_called()

    def test_dmg_verification_recovers_from_transient_lock(self):
        busy = subprocess.CalledProcessError(1, "hdiutil", output="Resource temporarily unavailable")
        with patch.object(n.subprocess, "check_output", side_effect=[busy, "verified\n"]) as command, patch.object(n.time, "sleep") as sleep:
            self.assertEqual(n.verify_dmg(self.out / n.DMG), "verified")
        self.assertEqual(command.call_count, 2)
        sleep.assert_called_once_with(2)

    def test_dmg_verification_bounds_persistent_lock_retries(self):
        busy = subprocess.CalledProcessError(1, "hdiutil", output="Resource busy")
        with patch.object(n.subprocess, "check_output", side_effect=busy) as command, patch.object(n.time, "sleep") as sleep:
            with self.assertRaises(subprocess.CalledProcessError):
                n.verify_dmg(self.out / n.DMG)
        self.assertEqual(command.call_count, 5)
        self.assertEqual([call.args[0] for call in sleep.call_args_list], [2, 4, 8, 16])

    def test_dmg_verification_does_not_retry_corruption_or_timeout(self):
        for error in (subprocess.CalledProcessError(1, "hdiutil", output="image checksum is invalid"),
                      subprocess.TimeoutExpired("hdiutil", 300)):
            with patch.object(n.subprocess, "check_output", side_effect=error) as command, patch.object(n.time, "sleep") as sleep:
                with self.assertRaises(type(error)):
                    n.verify_dmg(self.out / n.DMG)
                command.assert_called_once()
                sleep.assert_not_called()

    def test_dmg_trailer_checked(self):
        self.assets()
        (self.out / n.DMG).write_bytes(b"not a dmg" * 100)
        with self.assertRaises(ValueError):
            n.checksums(self.out, create=True)

    def test_existing_draft_or_published_release_refused(self):
        for draft in (True, False):
            with patch.object(n, "run", return_value=json.dumps([[{"tag_name": n.VERSION, "draft": draft}]])), self.assertRaises(ValueError):
                n.no_release()
        with patch.object(n, "run", return_value='[[], [{"tag_name": "0.1.1"}]]'):
            n.no_release()
        with patch.object(n, "run", side_effect=RuntimeError("API failed")), self.assertRaises(RuntimeError):
            n.no_release()

    def publication(self, corrupt=None):
        self.assets()
        n.checksums(self.out, create=True)
        assets = [dict(id=index, name=name, size=n.fingerprint(self.out / name)[0], state="uploaded", digest="sha256:" + n.fingerprint(self.out / name)[1]) for index, name in enumerate(sorted(n.NAMES), 1)]
        draft = dict(id=10, tag_name=n.VERSION, draft=True, prerelease=False, assets=[])
        uploaded = dict(draft, assets=copy.deepcopy(assets))
        if corrupt == "digest":
            uploaded["assets"][0]["digest"] = "sha256:bad"
        if corrupt == "inventory":
            uploaded["assets"].pop()
        published = dict(uploaded, draft=False)
        calls = []

        def fake_run(*args):
            calls.append(args)
            return ""

        def download(command, stdout, **kwargs):
            asset = next(a for a in assets if command[-1].endswith("/" + str(a["id"])))
            stdout.write(b"corrupt" if corrupt == "bytes" else (self.out / asset["name"]).read_bytes())

        current = copy.deepcopy(uploaded)
        for asset in current["assets"]:
            asset["download_count"] = 1
        current["assets"].reverse()
        if corrupt == "draft-edited":
            current["draft"] = False
        if corrupt == "asset-replaced":
            current["assets"][0]["id"] += 100
        with patch.dict(os.environ, self.env), patch.object(n, "guard", return_value=self.tag) as guard, patch.object(n, "no_release") as absence, patch.object(n, "run", side_effect=fake_run), patch.object(n, "api", side_effect=[draft, uploaded, current, published, published]) as api, patch.object(n.subprocess, "run", side_effect=download):
            if corrupt:
                with self.assertRaises(ValueError):
                    n.publish(self.root, self.out, self.tag)
                self.assertFalse(any("edit" in c for c in calls))
            else:
                n.publish(self.root, self.out, self.tag)
                self.assertEqual(guard.call_count, 2)
                absence.assert_called_once()
                create = api.call_args_list[0].args
                self.assertEqual(create[:3], ("releases", "--method", "POST"))
                self.assertIn("draft=true", create)
                self.assertIn("target_commitish=" + self.sha, create)
                self.assertFalse(any("releases/tags/" in c.args[0] for c in api.call_args_list))
                self.assertTrue(any("--draft=false" in c and "--latest" in c for c in calls))
                self.assertFalse(any("--clobber" in c for c in calls))

    def test_publication_success(self):
        self.publication()

    def test_publication_integrity_failure_never_publishes(self):
        for corrupt in ("digest", "inventory", "bytes", "draft-edited", "asset-replaced"):
            with self.subTest(corrupt=corrupt):
                for path in self.out.iterdir():
                    path.unlink()
                self.publication(corrupt)

    def test_publish_requires_original_tag_object(self):
        with self.assertRaises(ValueError):
            n.publish(self.root, self.out, None)


if __name__ == "__main__":
    unittest.main()

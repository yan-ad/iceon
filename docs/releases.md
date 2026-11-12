# Native Iceon release packaging and signed migration feed

`.github/workflows/release.yml` builds the pure-Rust GPUI app for Apple Silicon
macOS and Windows x86. There are **no Linux jobs or Linux assets** in this workflow.
Same-commit reusable CI still gates both platform builds and publication.

## Asset contract

The configured version is currently `0.1.4`. Expected uploaded assets are:

```text
Iceon_0.1.4_aarch64.app.tar.gz
Iceon_0.1.4_aarch64.app.tar.gz.sig
Iceon_0.1.4_aarch64.dmg
Iceon_0.1.4_x86-setup.exe
Iceon_0.1.4_x86-setup.exe.sig
Iceon_0.1.4_i686.zip
latest.json
SHASUMS256.txt
```

GitHub adds source ZIP/tar archives and displays sizes, download counts and SHA-256
digests. Those values are computed for the new bytes, not copied from older releases.
`SHASUMS256.txt` covers every other uploaded asset; it cannot hash itself.

**Published `0.1.2` already has the earlier portable ZIP asset set.** Editing this
workflow does not change that release. The publisher intentionally rejects an
existing draft/published release and never overwrites its assets. Before another
publication, choose a new release version and update the workspace/lock versions,
changelog, exact workflow tag/input/concurrency defaults, helper `VERSION` and
corresponding test fixtures together. Do not move an existing published tag.

## Packages and installation

- **macOS:** ARM64 `Iceon.app`, bundle ID `ad.yan.iceon`, executable `iceon`, matching
  bundle versions, embedded resources and copied `.icns`. The bundle is ad-hoc
  signed, checked, archived and placed in a compressed DMG. The DMG is verified
  and mounted read-only to validate the bundle and Applications link.
- **Windows:** i686 PE32 `iceon.exe` inside a Unicode NSIS setup EXE, compiled with
  pinned NSIS 3.11. A normal Setup wizard includes the MIT license, directory
  selection, Start menu shortcut and registered uninstaller. It installs for the
  current user without elevation, defaulting to `%LOCALAPPDATA%\Iceon`.
  No WebView2/bootstrapper or JavaScript runtime is installed.

The Windows installer preserves the previous `Software\Yanuar\Iceon` path and
`Software\Microsoft\Windows\CurrentVersion\Uninstall\Iceon` identity in the
32-bit HKCU registry view, including custom paths. `/S /UPDATE` supports the older
updater; `/R /ARGS` supports requested restart. A bounded overwrite retry allows
an exiting old process to release its executable; it never kills arbitrary processes.
The uninstaller removes owned files/registration, not app data or unknown files.

The Windows runner gates artifact upload on a synthetic old-install upgrade,
reinstall and uninstall test with installed-payload hash/registry checks. This is
not a full installed legacy-client update test or a graphical launch test.

## Upgrading from native 0.1.2

0.1.2 has **no in-app updater**. Its users must manually install this release;
the signed migration feed is for older updater-enabled clients, not native 0.1.2.

- **macOS Apple Silicon:** quit Iceon, open the new DMG, and replace
  `/Applications/Iceon.app` (or the same location used previously). Bundle identity
  stays `ad.yan.iceon`; do not delete the application-data directory.
- **Windows portable:** quit Iceon and extract `Iceon_0.1.4_i686.zip`, replacing
  the previous `Iceon/iceon.exe`. The archive retains 0.1.2's exact portable layout.
  Alternatively run Setup; this creates a registered per-user installation.
  Setup does not automatically locate arbitrary unregistered portable folders;
  update existing shortcuts or remove the old executable manually if switching.
- Both use the same data location as 0.1.2: macOS
  `~/Library/Application Support/ad.yan.iceon`, Windows `%APPDATA%/ad.yan.iceon`.
  Preserve any explicit `ICEON_DATA_DIR` override as well. Installation directories
  and application-data directories are separate.

Historical 0.1.2 SQLite fixtures test preferences, catalog metadata and route
history before the first new-version save. Retired settings are discarded,
legacy optional embedded themes migrate to System, and old pins become inactive.
No downgrade compatibility is promised. Back up application data before upgrading.
These automated checks are not a real signed-package replacement/launch test on
both operating systems; native smoke verification remains a release prerequisite.

## Trust and older-client updates

`latest.json` contains `version`, UTC `pub_date`, notes and exactly four platform
entries: `darwin-aarch64`, `darwin-aarch64-app`, `windows-i686`, `windows-i686-nsis`.
Aliases share the exact payload URL/signature. URLs point to the canonical repo,
release tag and matching archive/installer. There is no Linux entry.

The `.sig` files are base64-encoded complete Minisign documents. They authenticate
the **final archive/EXE bytes** and the trusted-comment `version:X.Y.Z` field.
The independent verifier checks both payload and trusted-comment signatures before
accepting the exact stable version. A JSON version alone is insufficient.

The standalone Rust signer/verifier lives in `crates/iceon-xtask/src/update_signing.rs`.
Its pinned public key is the one embedded in the updater-enabled older Iceon builds.
No old frontend/runtime or signing CLI is required. The publication step maps the
existing GitHub secret `TAURI_SIGNING_PRIVATE_KEY` into `ICEON_UPDATE_PRIVATE_KEY`
and its password secret into `ICEON_UPDATE_PRIVATE_KEY_PASSWORD`. An explicitly
empty password is supported. These legacy GitHub secret names preserve the trust
anchor; do not generate a replacement key to make a failure pass. Secrets are used
only for signing, never passed in argv or published as assets.

```sh
# Secret values supplied through environment, never command arguments:
cargo xtask sign-update dist/Iceon_0.1.4_aarch64.app.tar.gz 0.1.4
cargo xtask verify-update dist/Iceon_0.1.4_aarch64.app.tar.gz.sig dist/Iceon_0.1.4_aarch64.app.tar.gz 0.1.4
```

Updater-enabled old clients use the existing endpoint:
`https://github.com/yan-ad/iceon/releases/latest/download/latest.json`.
The new feed/installer are designed for that protocol. Releases without embedded
updater trust (notably 0.0.1/0.0.2) still require manual installation; `<0.1.1` is not
a uniform compatibility group. Validate migration from a real old install on each
OS before claiming end-to-end compatibility or preserved application data.

**The new GPUI app still has no native automatic updater.** Migrating into it does
not promise subsequent self-updates. Catalog/preferences migration across storage
formats is also not provided by installer format or download signatures.

Minisign updater authentication is distinct from platform signing: macOS remains
**ad-hoc signed, not Developer ID signed/notarized**; Windows remains **without
Authenticode signing** and may trigger SmartScreen. Do not disable OS protections
globally. Installation/launch and credential/cluster behavior need native smoke tests.

## Publication guards

The workflow currently accepts only exact tag `0.1.4`, without `v`. Manual dispatch
requires its immutable remote tag to point to the workflow commit. The helper
checks all four package versions/lock entries and a nonempty changelog section,
records the raw annotated-tag object, and rechecks it before build/publication.
It never creates/moves tags or resumes existing releases.

Final payloads are transferred from macOS/Windows build jobs. The macOS publication
job tests the Rust signer, signs and independently verifies both payloads, generates
and verifies the feed, then computes checksums. It creates a fresh draft, uploads
exactly eight assets without clobbering, checks metadata/digests, downloads and
compares every byte, rechecks signatures/feed/checksums/tag identity, then publishes
stable/latest. Any mismatch fails closed; a failed draft requires manual inspection.
Only publication has `contents: write`.

## Verification

```sh
python3 -m unittest discover -s .github/scripts -p 'test_native_release.py' -v
cargo test --locked -p iceon-xtask
cargo clippy --locked -p iceon-xtask --all-targets -- -D warnings
```

NSIS is compiled with warnings as errors. Windows-only install/upgrade/uninstall
smoke tests are in `.github/scripts/test_windows_installer.ps1`; they must run only
on disposable runners. Unit fixtures are not production signatures or live migration proof.

Protocol references:
- [Updater protocol](https://v2.tauri.app/plugin/updater/)
- [Exact older verifier implementation](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.14.0/plugins/updater/src/updater.rs)
- [Minisign Rust implementation](https://github.com/jedisct1/rust-minisign/tree/0.9.1)
- [NSIS](https://nsis.sourceforge.io/Docs/)

## Native update checks

Use **Help → Check for Updates…** or the download icon beside global search.
Iceon explicitly checks the stable GitHub release feed, compares semantic versions,
and offers the fixed official Releases page. This is advisory metadata over HTTPS,
not authenticated installer verification or automatic installation. No installer is
downloaded or executed by this action. Checks have bounded response size and timeout;
failures are shown without changing application data.

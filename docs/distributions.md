# Additional distribution preparation

This is build/package preparation, not a new release. The immutable `0.1.5`
release workflow, asset/signature contract, tags, and existing release are unchanged.
AUR is intentionally skipped. Developer ID signing and notarization are excluded;
macOS packages retain ad-hoc integrity signatures needed by the native app.

macOS packaging requires a macOS 26+ host and Xcode 26+ with `xcrun actool` available. Both
packagers compile `assets/icons/iceon.icon` into `Assets.car` and register its
compiler-generated icon metadata, retaining the ICNS fallback for older systems.
Bundled apps leave icon rendering to macOS so Default, Dark, Clear, and Tinted
appearances can follow the system setting. Unbundled `cargo run` uses a static
ICNS fallback; it cannot validate Icon Composer appearance switching.

### Icon Composer toolchain and diagnostics

The release ARM64 packaging job uses `macos-26`; distribution packaging uses
`macos-26` for ARM64 and `macos-26-intel` for Intel. These are explicitly listed
in the official [hosted runner image table](https://github.com/actions/runner-images#available-images).
Both build workflows explicitly set
`DEVELOPER_DIR=/Applications/Xcode_26.3.app/Contents/Developer` before compiling
Rust or packaging. The official [ARM64 macOS 26 image inventory](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-arm64-Readme.md#xcode)
and [Intel macOS 26 image inventory](https://github.com/actions/runner-images/blob/main/images/macos/macos-26-Readme.md#xcode)
list that installation (checked November 18, 2026). Do not rely
on the runner label to select Xcode. A missing pinned installation fails the job;
it does not fall back to an older compiler.

For a local full Xcode installation, set `DEVELOPER_DIR` explicitly and run:

```sh
python3 .github/scripts/macos_icons.py --check-toolchain
```

The same preflight runs on every icon compilation and reports the host macOS
version, resolved `actool` path, Xcode version/build, and actool bundle version.
It rejects older or unknown hosts before invoking Apple tools and requires Xcode
26+ and actool bundle version 24127+ (the shipping Xcode 26.0 compiler). Old tools
can return success for `.icon` input while producing only a partial plist; success
alone therefore never bypasses the required primary-icon metadata and nonempty
`Assets.car` checks.

The standalone invocation includes `--include-all-app-icons` and
`--lightweight-asset-runtime-mode enabled`. The latter uses Xcode's bundled asset
frameworks for consistent output, **not** a guarantee of compatibility with an
older build host. See the standalone `.icon` invocation in
[Godot's macOS exporter](https://github.com/godotengine/godot/blob/master/platform/macos/export/export_plugin.cpp)
and the runtime/version explanation in
[Chromium's icon compiler](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/tools/mac/icons/compile_car.py).
Chromium retains the enabled flag and explains the bundled-framework behavior;
it does not establish that disabling it on macOS 15 safely compiles Iceon's icon.
In failed release run `44242454682`, Xcode 26.3 / actool 24506 on `macos-15`
passed version preflight, then dyld reported missing CoreMedia/MediaToolbox
symbols in framework overrides. `AssetCatalogAgent-AssetRuntime` closed its
connection and compilation exited 255. This is evidence of host/runtime loading
failure, not an icon schema error or a reason to change the deployment target.
The focused compatibility policy is therefore to retain bundled-runtime mode
and package on macOS 26, including Intel, rather than retrying with an unverified
flag change or silently falling back to legacy-only icons. Non-compiling macOS
verification/export jobs and ordinary Rust CI can remain on macOS 15.

Chromium's undocumented `--enable-icon-stack-fallback-generation=disabled`
workaround is for pairing `.icon` input with existing `.xcassets` bitmaps; Iceon
does not supply that catalog and does not use that workaround. The existing
macOS 11.0 deployment target and separately packaged ICNS fallback remain intact;
the build-tool requirement does not raise the application's minimum OS version.
Apple's [Icon Composer documentation](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)
describes the layered file and app-icon naming contract, not these standalone CLI
flags. Mocked helper tests verify command construction and fail-closed behavior;
actual compilation and appearance switching still require supported Xcode and
macOS respectively.

## Manual workflow

After these changes are reviewed and pushed, run **Distribution preparation**
(`.github/workflows/distribution.yml`) on that reviewed commit. It has read-only
repository permissions and uploads Actions artifacts only. It never creates a
release, changes a tag, publishes a feed, pushes the tap, or uses signing secrets.
Flatpak is a separate opt-in input and is experimental.

| Runner | Rust target | Artifacts |
|---|---|---|
| Ubuntu 24.04 x86-64 | `x86_64-unknown-linux-gnu` | portable tar, amd64 `.deb` |
| Ubuntu 24.04 ARM64 | `aarch64-unknown-linux-gnu` | portable tar, arm64 `.deb` |
| macOS 26 ARM64 (`macos-26`) | `aarch64-apple-darwin` | app tar, DMG |
| macOS 26 Intel (`macos-26-intel`) | `x86_64-apple-darwin` | app tar, DMG |
| Windows x86 | `i686-pc-windows-msvc` | NSIS setup, ZIP |
| Windows 11 ARM64 | `aarch64-pc-windows-msvc` | ARM64 payload NSIS setup, ZIP |

The NSIS bootstrap is x86, including for the ARM64 package. Its payload is verified
as native ARM64; the ARM installer rejects other architectures and older Windows,
uses a distinct install identity and preserves the legacy x86 installation.
Installation/upgrade/uninstall must still be tested on real Windows ARM64.

Linux packages derive runtime requirements using `dpkg-shlibdeps` on their native
runner. No guessed dependency fallback is permitted. Ubuntu 24.04-built packages
are not a promise of compatibility with every Debian release: glibc and other
library minimums must match the generated dependencies. `.deb` downloads are not
an APT repository. Linux GPU/windowing, keyring and terminal runtime smoke tests
remain required before claiming feature parity. The portable tar does not bundle
all system libraries.

## Packaging locally

```sh
python3 .github/scripts/distribution.py build --target x86_64-unknown-linux-gnu
python3 .github/scripts/distribution.py package \
  --target x86_64-unknown-linux-gnu --version 0.1.5 --output dist/linux
python3 -m unittest discover -s .github/scripts -p 'test_*.py'
```

Use the checked-out workspace version and an empty output directory. Packaging
verifies binary architecture rather than trusting artifact filenames.

## Homebrew

The requested `yan-ad/homebrew-tap` exists and was empty when inspected. The manual
workflow generates `homebrew-tap-export-NOT-PUBLISHED` from the two actual DMGs,
checks their SHA256 independently, and validates Ruby syntax. See
[`packaging/homebrew/README.md`](../packaging/homebrew/README.md). It uses this
project's exact numeric release tags, **not** `v`-prefixed tags.

Do not push the generated cask until its exact DMGs have been published and the
URLs/checksums verified. The existing 0.1.5 asset set lacks an Intel DMG and must
not be silently replaced. A future expanded release needs a new version plus
reviewed publisher/feed/checksum contract changes. No tap credentials are needed
for local export; automatic tap updates later need a narrowly scoped GitHub App
or token with write access to `yan-ad/homebrew-tap`, stored as an Actions secret.
Homebrew runtime installation/audit and Gatekeeper behavior remain unverified.

## Flatpak

See [`packaging/flatpak/README.md`](../packaging/flatpak/README.md). The optional
workflow builds standalone bundles on x86-64 and ARM64 using the Freedesktop
25.08 runtime/SDK and Rust extension, with locked vendored Cargo sources and
network-disabled builds. It does not submit to Flathub or create a Flatpak remote.

The sandbox deliberately does not grant blanket host filesystem or host-spawn
permissions. Therefore host credential helper executables and host local shells
are not supported. Kubeconfig document-portal access, relative certificate paths,
keyring access, and reconnection persistence require native Linux testing. Do not
advertise full Iceon feature parity from a successful bundle build alone. A
future Flathub submission requires review and a separately approved application
manifest. No additional repository is required for the current bundle preparation.

## Validation boundary

Unit tests validate architecture/header checks, archive safety, metadata,
dependency parsing, checksum-backed cask generation and sandbox source handling.
Local macOS fixture packaging does not validate the real Linux or Windows app.
New target builds must pass the manual workflow after review; failures remain
blockers, not justification for unpinned dependency upgrades or security bypasses.

# Native Iceon 0.1.2 release

`.github/workflows/release.yml` packages the pure-Rust GPUI application in
[yan-ad/iceon](https://github.com/yan-ad/iceon), not the historical Tauri app.
It accepts only the exact tag `0.1.2` (no `v`). Manual dispatch requires that
existing remote tag to point to the dispatch's Actions commit. The workflow
never creates, moves or pushes a tag and never resumes/overwrites a release.
The raw remote tag object is recorded and rechecked before builds and publication,
including annotated-tag identity, not just the peeled commit.

## Prerequisites and gates

Before tagging, maintainers must set `[workspace.package].version`, any explicit
local package versions, and all four local Cargo.lock packages to `0.1.2`, and
add a nonempty `## 0.1.2` changelog section (optionally ` (pending CI)`). The
helper fails closed on any mismatch. The workspace and local packages are aligned
to `0.1.2`, following the latest published release, `0.1.1`; the previously
unpublished workspace version `0.1.8` was corrected during release preparation.

The reusable `ci.yml` runs at the workflow commit before packaging; both platform
builds and publication depend on its success. Only publication has
`contents: write`. Checkout is the Actions commit, not a separately resolved tag.

Native build commands use `cargo build --release --locked -p iceon-app --target`:

- `aarch64-apple-darwin` on ARM64 `macos-15`: creates `Iceon.app`, sets bundle
  identifier `ad.yan.iceon`, executable `iceon`, and both bundle versions to
  `0.1.2`. Copies `assets/icons/icon.icns` if available, ad-hoc signs the bundle,
  verifies signature/ARM64 metadata and executable permissions, creates a tarball
  and compressed DMG. The DMG is verified and mounted read-only to inspect the
  signed app, binary identity and `/Applications` link.
- `i686-pc-windows-msvc` on `windows-2022`: initializes ilammy's x86 MSVC environment
  and the existing absolute x86 linker action. Packages `Iceon/iceon.exe` in a
  standalone ZIP and validates its PE32/i386 headers and ZIP integrity. There is
  no NSIS, installer, WebView2 or external installer-tool dependency. Windows
  system/runtime DLL availability and real launch behavior need native smoke tests.

### Embedded assets

No source-tree asset directory is required beside the executable. Existing
`crates/iceon-ui/src/app_assets.rs` embeds the welcome PNG with `include_bytes!`,
embeds the additional Lucide icons with `gpui_kit_assets::icon_assets!`, and falls
back to `gpui_kit::assets::Assets`. `lib.rs` installs this asset source at startup.
The workspace enables gpui-kit 0.9.0's default `assets` feature. Console fonts are
embedded by `console_font.rs`; GPUI kit supplies its own bundled component assets.
The locked GPUI Apple renderer also embeds `shaders.metallib` via `include_bytes!`
in gpui-pre-apple 0.3.11, rather than loading a sidecar Metal library.
The normal existing app build includes these assets: no Rust/manifest changes or
Tauri resource-copy step are necessary. Existing Rust asset tests run in reusable
CI. The optional `.icns` is the native bundle icon, separate from embedded UI assets.

## Publication and installation

Exactly four public assets (GitHub-generated source archives are separate):

| Asset | Purpose |
| --- | --- |
| `Iceon_0.1.2_aarch64.dmg` | Apple Silicon manual install; drag app to Applications |
| `Iceon_0.1.2_aarch64.app.tar.gz` | Same signed app bundle, manual extraction/install |
| `Iceon_0.1.2_i686.zip` | Windows x86 standalone executable; extract before running |
| `SHASUMS256.txt` | SHA-256 of the three packages |

The publisher validates transferred archives, exact asset inventory and checksums
before creating a fresh draft. It uploads without `--clobber`, checks remote
names/sizes/states and digests when provided, downloads all four assets and compares
their bytes, revalidates checksums/archives, and rechecks source/tag guards before
publishing stable/latest. It confirms the published and latest release identity.
An API error is not treated as release absence. A failed upload/verification leaves
a draft for manual inspection; reruns refuse it, rather than replacing assets.
GitHub repository release immutability, if enabled, takes effect on publication;
workflow checks also protect the existing tag throughout this run. Administrators
must still prevent concurrent external tag/release edits; API operations are not
an atomic transaction or a repository tag-protection rule.

The macOS app is **ad-hoc signed, not Developer ID signed or notarized**. Windows
is **unsigned** and may trigger SmartScreen. Checksums verify integrity, not
publisher identity. Do not disable OS security protections globally. Native
installation, Gatekeeper/SmartScreen, graphical startup and credential/cluster
integration require manual smoke tests on the supported platforms.

There is **no native updater integration**, no `latest.json`, no updater signatures,
and **no claim of Tauri updater compatibility**. Existing Tauri installations
cannot use these archives as an automatic update. Install manually. Historical
release workflows and updater documentation are not the native release contract.

## Helper tests and API references

Run without third-party Python packages (Python 3.11+):

```sh
python3 -m unittest discover -s .github/scripts -p 'test_native_release.py' -v
```

References checked for this workflow:

- [Reusable workflows](https://docs.github.com/en/actions/sharing-automations/reusing-workflows)
- [Upload artifact](https://github.com/actions/upload-artifact) and
  [download artifact](https://github.com/actions/download-artifact) (`pattern`, `merge-multiple`)
- [MSVC environment](https://github.com/ilammy/msvc-dev-cmd) (`arch: x86`)
- [gh release create](https://cli.github.com/manual/gh_release_create) (`--draft`, `--verify-tag`)
- [gh release upload](https://cli.github.com/manual/gh_release_upload) (no `--clobber`)
- [gh release edit](https://cli.github.com/manual/gh_release_edit) (`--draft=false`, `--latest`)

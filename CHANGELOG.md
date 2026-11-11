# Changelog

## 0.1.3

### Native installers and signed migration feed

- Replace the Windows portable ZIP with a per-user x86 NSIS setup EXE, a Start
  menu shortcut, registered uninstaller and preservation of the legacy custom
  installation path. Gate packaging with synthetic upgrade/reinstall/uninstall tests.
- Publish signed macOS application archives and Windows setup EXEs, matching
  `.sig` files, `latest.json` and SHA-256 checksums. Use a standalone Rust signer
  and independent verifier with the existing public trust key and authenticated
  release-version comments; no frontend or old application runtime is required.
- Restore the download/feed format expected by compatible older updater-enabled
  clients. Pre-updater installs still require manual installation; end-to-end
  migration remains subject to installed-client testing. The native GPUI app
  does not yet update itself after migration.
- Run release jobs only on macOS and Windows. Preserve same-commit CI, immutable
  tags, fresh-release publication and uploaded/downloaded-byte integrity gates.
- macOS remains ad-hoc signed, not notarized; Windows setup is not Authenticode
  signed. Updater signatures do not replace platform signing.

### Project documentation and license

- Align the project LICENSE, Cargo workspace/crate metadata and README on MIT,
  retaining third-party licenses and notices.
- Refresh native architecture, feature/OS roadmaps and icon ownership documentation;
  remove obsolete web-frontend documentation, workflows and updater tooling.

## 0.1.2

### Native Rust desktop

- Deliver a pure-Rust GPUI desktop application and
  Cargo workspace. Add native themes, settings, a welcome view, workspace
  breadcrumbs, and resizable navigation and resource tables.
- Improve cluster catalog import, metadata columns, namespace navigation,
  search, cluster health probes, and explicit health refresh controls.
- Add native local terminals, pod logs and console docks, kubectl/Helm tool
  preferences, and YAML resource editing while preserving guarded mutations
  and metadata-only handling of sensitive resources.

### CI and distribution

- Focus native CI on macOS and Windows; Linux builds are deferred.
  Fix strict Rust/Clippy diagnostics across the core and native UI without
  weakening warning enforcement.
- Use same-commit CI-gated native
  Apple Silicon DMG/application archives and a portable Windows x86 ZIP.
  Validate package architecture, bundle metadata, immutable tag identity,
  checksums, and uploaded/downloaded bytes before publication.
- This is a manual-install native release. macOS packages are ad-hoc signed,
  not notarized; the Windows ZIP is unsigned. Automatic updates
  are not integrated, and no updater manifest is published. See `docs/releases.md`
  for installation and verification instructions.

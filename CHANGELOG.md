# Changelog

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

# Changelog

## 0.1.8

- Fix preference-store conflicts when changing terminal, editor, or application
  fonts after another preference writer has committed changes.
- Apply individual Settings changes transactionally to the latest persisted
  preferences, preserving unrelated concurrent changes and integration rollback.
- Prevent delayed background-import completions from reverting newer font and
  preference state in the main window or Settings.
- Add isolated regression coverage for stale revisions, concurrent preference
  changes, validation failures, and persistence/integration rollback.

## 0.1.7

- Add native extension foundations and built-in extension navigation, a marketplace,
  and Zed icon theme support.
- Follow the operating-system accent for selected sidebar labels and resource links,
  with readable contrast in light and dark modes.
- Add collapsible sidebar parents, text-only children, and an icon-only rail that
  preserves expanded width; include an offline native interaction probe.
- Use system-owned macOS Icon Composer appearances in packaged apps instead of
  overriding the Dock with a flattened icon; compile layered assets during packaging.
- Add automatic update download and restart flow, safe ConfigMap editing, and
  improved empty states and table rendering.
- Fix Windows accent observer compilation and remove obsolete sidebar icon code.

## 0.1.6

- Add cached, searchable and sortable Helm chart/release tables and safe metadata
  inspectors with revision history; values, notes and manifests are not fetched.
- Support Helm 3.17+ and Helm 4 install/upgrade command contracts, explicit
  all-status release listing, empty repository inventories and safe error handling;
  reject unsupported mutation versions and options without silently downgrading.
- Show explicit inventory counts in the header, including filtered and partial
  results, and refine resource-reference inspector toggles and last-applied YAML.
- Keep RBAC ServiceAccount planner controls bound to their opening route, cluster,
  session and namespace scope so stale callbacks cannot act after navigation.
- Improve shell readability with more opaque chrome and content surfaces while
  retaining native blur on the intended chrome surfaces.
- Add separate, manual build-only multi-platform distribution preparation, including
  optional experimental Flatpak bundles. Stable release assets remain Apple Silicon
  macOS and Windows x86; no expanded platform publication or signing is claimed.

## 0.1.5

- Add guided cluster setup, connection diagnostics and recovery, and multi-namespace
  selection with improved namespace navigation.
- Add redacted pod inspection plus Deployment and DaemonSet inspectors and guarded
  workload actions; enforce custom-resource access reviews and add an RBAC users table.
- Improve Quick Jump across catalog clusters, add Kubernetes API brand icons,
  and support multi-status resource filtering.
- Consolidate console session controls and tab navigation; add a device-terminal
  shortcut and refine Overview, compact badges, borders, and native window blur.
- Register native macOS `iceon://create-cluster` and `iceon://settings` actions for
  kubeconfig import and settings. Core Spotlight indexing is not implemented.

## 0.1.4

- Refresh the native shell with compact header controls, a sidebar footer, and
  footer-aligned collapsed terminal controls; expanded terminal tabs stay on top.
- Add data-only Zed marketplace theme management, unified theme settings, and
  native file/folder/pasted-YAML cluster configuration.
- Improve scoped RBAC navigation, automatic namespace discovery, initial connection
  feedback, background refreshes, and persistent access-time Kubernetes versions.
- Cache Overview snapshots and bound its Events preview; add container indicators
  and pod usage columns, consistent empty states, and status-colored Events rows.
- Add console tab context menus, focus-aware close shortcuts, shared YAML dock
  creation/editing, and guarded bulk resource deletion.
- Add explicit update checks with manual release downloads; no automatic installer
  execution. Preserve 0.1.2 user-data migration and Windows portable ZIP upgrades.
- Retain signed legacy migration feeds, release integrity gates, macOS ad-hoc signing
  and Windows unsigned platform packages; no notarization/AuthentiCode claim.

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

# Iceon design

## Product and implementation status

Iceon is a native Kubernetes workspace for developers and operators. The desktop
application is Rust end to end, using `gpui-kit = "0.9.0"` for its interface and
Rust services for cluster access, persistence and process ownership. There is no
web frontend, browser-rendered desktop UI, JavaScript runtime or frontend IPC layer.

[Freelens](https://github.com/freelensapp/freelens) inspires exploration and
operational workflows, not the implementation architecture. Iceon does not load
Freelens extensions or promise feature/API compatibility. A native extension model
and workflow portability remain research; an Extensions information view is not
an extension host.

The source implements catalog/import, navigation, cluster health, resource tables,
inspectors, YAML, Pod consoles, forwarding, Helm workflows, permission services,
preferences and a separate Secrets workspace. A core service does not prove that
every operation has a complete native UI. Live-cluster acceptance, native packaged
smoke tests and broader OS coverage remain necessary. The compact status and
roadmaps live in [README.md](README.md).

## Workspace architecture

| Crate | Responsibility |
| --- | --- |
| `crates/iceon-app` | Binary entry point; calls `iceon_ui::run()` |
| `crates/iceon-ui` | GPUI views/entities, actions, native menus, dialogs, input and presentation |
| `crates/iceon-core` | Catalog, connection/session services, preferences, route history, Secrets, TLS, shared runtime and process services |
| `crates/iceon-xtask` | Cargo maintenance/release utilities; current shipping contract is `docs/releases.md` |

The root `Cargo.toml` owns workspace dependencies, version and profiles.
Application code lives beneath the crate directories, not a root `src/` directory.

`iceon_ui::run()` initializes TLS, enters the shared Tokio runtime, constructs the
engines, initializes GPUI/assets/fonts/actions, then opens the main window.
`IceonApp` holds shared engines and presentation state. GPUI entities own input,
editors, tables and console surfaces. Asynchronous completions must check current
session/request identity before updating the active view.

Kubernetes/network/filesystem/process work belongs in core services. Views call
typed Rust APIs and render bounded projections; they must not invent successful
operations, replace authorization with button visibility, or expose raw credentials.
GPUI foreground futures run outside Tokio's executor, so background work uses the
shared runtime explicitly rather than assuming a thread-local executor.

### Development transport

`iceon-core/src/dev_engine.rs` and the `iceon-dev-engine` binary retain an opt-in,
debug-only HTTP transport for engine development. It is not the desktop renderer
or a supported browser UI. It has host/origin/token checks, bounded messages and
an expiring owner lease. The normal developer entry point is `cargo run -p iceon-app`;
there is no frontend dev server. Tests use synthetic configs and temporary homes.

## Visual direction

The native interface uses compact GPUI components, neutral content surfaces,
restrained separators and a blue brand accent (`0x1a43bf`). Semantic success,
warning and danger colors retain their meaning rather than all becoming blue.
Light/Dark/System appearance and editor/terminal choices come from preferences.

`shell_theme.rs` is the shared theme application path. Window/root chrome stays
transparent; the main content paints its own background. macOS uses the native
blurred window appearance, while other platforms use compositor transparency when
available. Do not simulate platform support or claim identical material effects.
Theme changes should not rebuild unrelated editors, streams or sessions.

### Layout anatomy

- **Shell header:** navigation/history actions, cluster selection and breadcrumbs;
  actions operate on the current destination, not an unrelated background view.
- **Cluster rail and sidebar:** pinned cluster shortcuts and cluster-local resource
  navigation; widths/collapse state are explicit presentation state.
- **Main pane:** Catalog, resource inventory, overview, Secrets or preferences-related
  content. Tables retain predictable column/selection/scroll ownership.
- **Inspector:** a distinct detail view for the selected resource with refresh,
  metadata, facts, related resources/events and guarded actions.
- **Bottom workspace dock:** GPUI Kit `DockArea`/`DockSkin` console sessions, with
  collapse, resize and maximize controls; navigation and inspector are not fake docks.

Primary owners: `shell_header.rs`, `sidebar_view.rs`, `catalog_view.rs`,
`cluster_view.rs`, `resource_table.rs`, `inspector_view.rs` and `workspace_dock.rs`
in `crates/iceon-ui/src/`. [Workspace reference](docs/workspace-ui-reference.md)
maps the current native surfaces.

### Interaction and feedback

Catalog activation opens a workspace and the deliberate connection lifecycle;
pinning and row actions must not accidentally activate a row. Resource navigation
preserves cluster identity and uses explicit namespace scope. Back/forward and
breadcrumbs are validated routes, not arbitrary executable strings.

Search/filtering describes the loaded inventory, not a complete cluster query.
Paged/partial results, absent metrics, permission denial and stale data remain
visible. Initial loading, empty success, filtered-empty, refresh failure and retained
rows are distinct states. A refresh must not reset user focus or spawn duplicate
streams. Tables use dedicated pagination/scroll/age helpers rather than rebuilding
the workspace for each row or clock tick.

Destructive actions require explicit review and confirmation. Enabled controls are
only affordances; the engine rechecks request identity, permission and preconditions.
Dialogs must distinguish server dry-run from a real mutation and present conflicts
without silently retrying destructive work.

## Engine boundary and safety invariants

### Catalog and destination pins

`iceon-core/src/catalog.rs` and `catalog/` own bounded metadata import, SQLite
storage, source identity, ordering, colors, pins and source reconciliation. Only
cluster/context metadata belongs in the catalog. Never persist kubeconfig tokens,
private keys or certificates in SQLite, logs or view models.

Import itself does not execute credential helpers or contact the cluster.
Connecting may run the selected configured helper, so imported kubeconfigs must
be trusted. Default/configured source reconciliation is separate from an explicit
manual import. Unavailable sources must not be confused with confirmed removals.
Pins identify validated cluster/destination routes; they are not credentials or
permission grants. See [platform discovery](docs/platform-discovery.md).

### Connection, cancellation and credentials

`iceon-core/src/connection.rs` owns active session leases. Closing or replacing a
workspace cancels owned operations, watches and streams through cancellation tokens;
stale completions cannot write into a new session. Reads and long-running services
have explicit ownership and bounds. Cleanup must kill/reap owned processes and close
streams, not merely hide their controls.

`connection/config.rs`, `auth.rs` and `renewal.rs` own selected-context client setup,
HTTPS/CA validation and supported credential renewal. No global insecure-TLS bypass
or hidden cloud-login flow is implied. Session and identity checks remain valid
through renewal. Public errors must not echo helper environment, raw kubeconfig,
tokens, private keys or server responses. See [credential renewal](docs/credential-renewal.md).

Shutdown closes local terminals, native tray integration, connection and Secrets
services, then performs a bounded drain. OS integration failures are surfaced rather
than pretending a setting took effect.

### Resource inventory, scope and sensitive projections

`ResourceKind` is an explicit allowlist; resource routes map to typed requests.
Namespaced and cluster-scoped inventory semantics are not interchangeable.
`cluster_view.rs` owns native navigation/loading, `resource_table.rs` the table
entity, and core connection modules the network operations/projections.

Pod LIST/WATCH synchronization has an owned lifecycle (`connection/pod_sync.rs`).
Do not claim all resources have equivalent live-watch behavior. Pagination and
partial flags describe the actual loaded window. Namespace cache, metrics and
custom discovery have their own bounds and errors.

Secrets and ConfigMaps expose safe metadata projections rather than their values.
Custom-resource browsing is metadata-first and does not authorize arbitrary secret
payload rendering. [Custom resources](docs/custom-resources.md) describes the scope.

### Inspection and YAML

`connection/details.rs`, `detail_facts.rs`, `detail_metadata.rs` and
`detail_collections.rs` project resource identity, facts and related items.
`inspector_view.rs` renders these projections; `editor_surface.rs` and `yaml_dock.rs`
own the native editor surfaces and tabs. Syntax highlighting uses the GPUI Kit
YAML integration, not browser editor libraries.

YAML is an operational surface, not permission to dump redacted data. Resource
creation/edit/deletion and workload/node actions remain engine-owned. Mutations
validate UID/resourceVersion where applicable and support server dry-run. New
resource scaffolds are starting points, not guaranteed valid manifests. Server
schema, admission and RBAC remain authoritative.

### Permissions and managed access

`connection/permissions.rs` owns self-access observations; `rbac.rs` projects
observed subjects and `rbac_management.rs` owns reviewed management plans.
Kubernetes has no generic User object: an observed RBAC subject is not proof of a
login account. Managed ServiceAccounts and assignments are constrained workflows,
not an unrestricted identity provider or arbitrary privilege editor.

Permission checks can expire or fail; the server always makes the final decision.
Plans and applies bind to the current session and resource identities, support
review/dry-run and report partial/conflict outcomes. Never imply atomic rollback
of a multi-object operation. See [permission management](docs/permission-management.md).

### Node drain

`connection/node_drain.rs` implements conservative HTTP eviction with review,
permission/precondition checks, cancellation and a short-lived engine-owned plan.
It is not a force-delete shortcut or a generic `kubectl drain` wrapper. Blockers,
partial inventory and disruption-budget failures must remain explicit. Existence
of the core workflow does not establish a complete native drain wizard.

### Helm and forwarding

`connection/helm.rs` and related modules supervise bounded Helm CLI operations;
preferences select the executable/proxy behavior. Catalog/repository data is not
cryptographic chart verification. Install and other mutations remain explicit,
reviewable and session-owned. A successful dry-run does not grant future authority.

Pod/service forward services bind loopback and own listener/task shutdown. Closing
or replacing a session cancels its forwards. Do not expose arbitrary external
listeners or mistake a retained table row for a running tunnel.

### Pod and device consoles

`pod_dock_view.rs`, `terminal_surface.rs`, `local_console.rs` and `workspace_dock.rs`
render native console sessions. Remote exec/attach/log services live in
`connection/pod_runtime/`; local device shells use `iceon-core/src/local_terminal.rs`
and `portable-pty`. A device shell uses the local user identity, never a Pod shell
preference. Merely rendering an empty Terminal action must not spawn a process.

The console uses bounded retention/session counts and a stateful `vt100` terminal
parser. Input/output changes invalidate the owning surface rather than all panes.
Logs support explicit presentation/filter/export controls; exported data is retained
output, not a claim of complete server history. See [Pod console](docs/pod-console.md).

### Secrets workspace

`iceon-core/src/secrets/` and `iceon-ui/src/secrets_view.rs` are separate from the
cluster catalog. Vault userpass and read-only KV v2 are the initial provider scope.
Profiles persist connection metadata; saved issued tokens use OS credential storage,
not plaintext fallback. Passwords are not persisted.

Secret values require explicit Reveal, expire after 30 seconds, and are cleared on
navigation/blur/disposal. Cancellation and session identity isolate connections.
No remote secret writes, implicit credential injection or unsupported provider
operations should be advertised as implemented.

## Assets and native integration

`assets/icons/icon.png` is embedded by `app_assets.rs` for native UI branding;
`assets/icons/128x128.png` is used by the README, and `32x32.png` is embedded
by the macOS tray implementation. The macOS package copies
`assets/icons/icon.icns` as its bundle icon. `icon.ico` is available for Windows
branding but the current portable packager does not embed it in the executable.
See [icon inventory](assets/icons/README.md).

Component/extra Lucide icons and console fonts are embedded by Rust asset sources.
There is no runtime frontend asset-copy/build step. Third-party font/theme notices
remain with those assets.

`native_menu.rs`, `native_app.rs`, `tray.rs` and core `app_integration.rs` implement
platform integration. Dock/status-item and login integration are macOS-specific;
unsupported platforms must not show a successful action or enable unsupported
settings. Window controls, menus and accelerators use GPUI/platform APIs.

## Distribution and OS support

The authoritative native shipping contract is [docs/releases.md](docs/releases.md).
`release.yml` gates packaging on same-commit macOS/Windows CI, produces Apple
Silicon DMG/app archives and a Windows x86 portable ZIP, verifies checksums and
uploaded/downloaded bytes, then publishes only a new verified release.

macOS packages are ad-hoc signed, not notarized. Windows packages are unsigned.
There is no integrated automatic updater or update feed. Downloads/checksums do
not replace publisher signing or native smoke tests. Linux Rust CI/packaging is
currently deferred; Intel macOS and Windows x64/ARM64 have no release targets.
Do not equate compilation or packaging with verified installation/runtime support.

## Verification

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 -m unittest discover -s .github/scripts -p 'test_native_release.py' -v
```

Tests should use temporary stores, injected homes, synthetic credentials and local
fixtures. No test should discover a developer's real credentials or mutate a live
cluster by default. Native acceptance separately covers focus/keyboard behavior,
window sizing/materials, accessibility, credential prompts, installed launch,
quit cleanup and test-cluster permission/conflict/cancellation paths. Unit tests
alone do not certify those behaviors.

## References

- [Workspace UI](docs/workspace-ui-reference.md)
- [Resource inspector](docs/resource-inspector-reference.md)
- [Preferences](docs/preferences.md)
- [Pod console](docs/pod-console.md)
- [Native distribution](docs/releases.md)
- [Freelens](https://github.com/freelensapp/freelens): workflow inspiration only

# Iceon design

## Browser development transport

`bun run tauri:inspect` starts Vite's opt-in Rust development bridge alongside the
desktop runtime. Browser presentation uses typed engine adapters, not fake Tauri
globals; native chrome, dialogs, menus, updater, and OS openers remain desktop-only.
The bridge uses an ephemeral IPv4 loopback listener behind Vite, exact origin/host
validation, a per-run server-side bearer token, bounded requests/responses, and a
single-tab workspace lease with heartbeat expiry. Release builds cannot include it.

Inspect and `dev:web` discover only `~/.kube/config`, through Rust's existing
metadata-only discovery on Catalog load/Refresh. Browser storage is isolated in
`.iceon-dev/`; its connections are independent of the desktop engine. Workspace
entry and confirmed cluster operations retain their existing live effects.
Tests inject temporary discovery homes and synthetic kubeconfigs, never real
credentials. Acceptance coverage: `tests/dev-engine.spec.ts` and Rust
`dev_engine::tests`; this development workflow has no upstream UI reference.

## Product and objective

Iceon is a desktop Kubernetes workspace for developers and operators. Freelens
is the reference for user-visible workflows, not the architecture or a promise of
complete compatibility. Keep Iceon's Astryx interface rather than copying
Freelens's styling or Electron implementation.

The objective is to discover clusters, connect deliberately, inspect resources,
and perform operational work without losing cluster or namespace context.
Behavioral targets include:

- A cluster-only Catalog, with search and pinned cluster/workspace shortcuts.
- Kubeconfig/context discovery and import, explicit connect/disconnect states,
  and actionable authentication, TLS, network, and permission errors.
- Cluster-scoped resource tables, namespace/filter selection, details, events,
  YAML inspection/editing, and confirmed mutations.
- Logs, exec/terminal sessions, port forwarding, metrics, and Helm workflows as
  separately delivered features, each with cancellation and cleanup.

These are objectives, not a shipped-feature list. Today the app implements the
shell, Catalog as the home screen, category search, navigation history, and the
seamless window frame. The first Rust catalog engine persists imported cluster
context metadata, individual cluster/section pins, and random record colors in SQLite.
Built-in discovery imports `~/.kube/config` on catalog load/Refresh.
Background file watching is not implemented. The connection foundation now provides
Rust-owned session leases, automatic connection on workspace entry, bounded
HTTPS client setup, and read-only Nodes/Namespaces snapshots. Cluster rows open a compact
workspace with inset navigation and a cluster-local breadcrumb/status/actions row.
The workspace workflows include bounded resource inventories, Pod LIST/WATCH sync,
protected Pod details/YAML and confirmed mutations, logs, exec/attach, loopback
forwards, Helm chart/release inventory, and explicit Helm installation. Resource tables are ordinary
workspace navigation; there is no Resource tables toggle or preference. All 41 allowlisted
Kubernetes resource kinds expose metadata-scaffold YAML creation with server dry-run
and confirmation, plus checkbox selection and confirmed deletion. API availability,
schema validity and RBAC remain server-enforced; scaffolds are not complete manifests.
Do not claim Freelens extension compatibility or expose unfinished operations.

### Approved 0.0.2 permission work (implementation in progress)

Access Control > Users is approved as a managed ServiceAccount workflow, not a
directory-only substitute or normal human/IdP account CRUD. Select an explicit
user namespace, create an account with `automountServiceAccountToken=false`,
assign existing prefixed ClusterRole templates to selected namespaces or an
explicit all-namespaces scope, and choose separate None/Read/Write cluster access.
These labels select actual rules, not boolean effective-access claims. Review
exact rules and server dry-run results before confirming create, edit or delete.
No automatic namespace/role seeds, User CRD, token Secret or credential export
belongs to this release.

Rust owns the management plan, ownership and unrelated-subject checks,
UID/resourceVersion baselines and authorization epoch. A plan is single-use,
expires after 120 seconds and cannot survive a changed authentication epoch.
Partial execution stops future submissions without automatic rollback or replay.
Permission-aware navigation and menus remain scoped to supported operations;
Read-only requires known allowed reads and known denied relevant writes. Unknown
access is Access unverified, not a denial or a read-only guarantee. This is not a
claim that every workspace helper already participates in permission gating.

The implementation contract and acceptance criteria are in
[permission management](docs/permission-management.md). The broader
[RBAC design and pinned research](docs/permission-manager.md) retain future
phases, including full Role/Binding editing and separately approved credential
lifecycle work. SIGHUP Permission Manager at
`fb6f65e161ac792ab290ffe680d6420caf834a43` is a BSD-3-Clause workflow reference;
no source was ported and Iceon's GPL v3 license is unchanged. Combined release
validation, native checks and live-cluster verification are pending.

## Existing visual direction

The interface is a restrained operational workspace, not a marketing dashboard.
Its recognizable structure is one collapsible sidebar with categories and pinned items,
with compact, aligned resource rows as the main working surface.

| Decision | Direction and reason |
| --- | --- |
| Theme | `src/themes/brand.ts` extends Astryx `neutral`, with Azure generated by Astryx CLI from seed `#0074e2` in `src/themes/azure/`. Accent, on-accent, muted, text and icon roles use mode-specific palette stops. Neutral surfaces, green success states and black log surfaces remain distinct. The installed Matcha package is not an active theme. |
| Color | Theme tokens for surfaces, dividers, text, focus, selection, and semantic state. No page-specific palette or root color overrides. |
| Typography | Astryx theme typography for navigation and reading. Monospace is for YAML, logs, commands, and identifiers where character alignment matters. |
| Structure | `AppShell variant="section"`, with dividers rather than floating cards, keeps navigation distinct without wasting working space. |
| Density | Compact tables and row lists make cluster inventories scannable. Catalog Status cells use Astryx neutral badges as explicitly requested; Labels/source metadata stay plain text. |
| Spacing | Preserve the existing token rhythm: tight toolbar groups, moderate navigation gaps, and roomier page headers/empty states. Current content uses padding step 6 and gaps 4/5. |
| Icons | Tabler icons express concrete actions and resource categories. Icon-only controls always have accessible names and tooltips. |
| Motion | Feedback and navigation transitions only; exiting cluster navigation is inert and accessibility-hidden. Reduced motion disables transitions; no decorative animation while users inspect resources. |

Design dials: **ENERGY 1 / RHYTHM 1 / MOTION 1**. Predictable placement supports
repeated operational work; variation follows content, not decoration.

`src/main.tsx` owns reset/base styles. `PreferencesProvider.tsx` owns the controlled
root Astryx `Theme` inside the preferences context, using the persisted Light,
Dark or System mode. Preferences drafts apply only after Save, not as live previews.
Initial preferences load asynchronously; this is not a no-flash startup guarantee.
`src/App.css` is
unused starter CSS, not design authority. The active brand theme lives in
`src/themes/brand.ts`; new palettes, modes, and fonts belong in the theme.

## Layout anatomy

1. **Window frame:** device insets and a single toolbar with automatic native-control
   clearance when seamless is enabled. No app-name strip or extra title-bar row.
2. **Toolbar:** Back/forward history, then Refresh immediately after
   Go forward. Cluster routes place the Switch cluster selector after Refresh.
   History controls disable when unavailable. Search opens from its icon button
   or Cmd/Ctrl+K. Preferences and Search use named icons and move into overflow at
   narrow widths. Add config
   is a named icon action at the Catalog's bottom right, outside its table scroll
   region; it is not duplicated in the toolbar or a Catalog actions menu.
    Empty space between actions is draggable in native seamless mode.
    The GitHub icon immediately before More invokes Rust `project_open_repository`
    with no renderer URL argument. `src-tauri/src/project.rs` opens only
    `https://github.com/yan-ad/iceon`; browser preview disables the action and
    native failures return a safe error. This does not broaden arbitrary opener access.
3. **Single sidebar:** No separate app rail, Home button, or Welcome destination.
   `/` redirects to `/catalog`. Catalog navigation is always icon-only; there is
   no toolbar collapse/expand control and the legacy collapse preference is no
   longer read or written. Narrow windows retain this icon sidebar with a local
   Open sidebar action for the existing drawer and readable cluster navigation.
   Pinned shortcuts identify individual clusters or cluster-scoped workspace
   destinations, never the Clusters catalog page.
   Opening `/cluster/<id>`, returning to Catalog, reloading and changing window
   size all retain icon-only Catalog navigation. The inset cluster navigation
   remains readable on desktop and available through the drawer on narrow windows.
4. **Sidebar content:** A single Clusters entry and a pinned-items area share one
   navigation landmark. Collapsed mode keeps named category and pinned-entry
   icons usable, with labels available to assistive technology and tooltips.
   Clusters is navigation only. Pin actions belong to loaded list
   entries and static cluster-navigation rows, including grouped sections.
5. **Content:** one main landmark and an accessible, visually hidden page heading.
   No category title bar, discovery switch, or routine explanatory information
   block; the normal view begins with the filter and table. Keep actionable errors
   and loading/empty states visible rather than hiding failures for appearance.
   Resource inventories use
   compact tables; explanatory/empty states use text and specific actions.
   Every category uses `Name | Kind | Source | Labels | Status | Actions`.
   Kind uses the category name and matching sidebar icon; Source identifies
   `kubeconfig` or `local`; Labels show where metadata was loaded, such as
   `file=~/.kube/config` or `file=/absolute/path/config`. Disconnected remains
   neutral; known positive resource/connection states use semantic success tokens,
   failures use error, and pending/degraded states use warning. Unknown text and
   generic booleans stay neutral, never an invented healthy/connected state.
   Each entry has a category glyph on a random persisted hex-color background,
   with a black/white glyph chosen by WCAG relative luminance for contrast. Colors
   identify entries only, never health or category; category text remains visible.
   This runtime data color is a deliberate exception to static theme-token colors.
   `/catalog` and `/catalog/clusters` show the same cluster-only view. Legacy
   `/catalog/all`, `/catalog/general`, and `/catalog/web-links` URLs redirect to
   `/catalog/clusters`; they are not navigation or search entries.

Catalog cluster badges are runtime observations, not persisted import metadata.
Rust overlays the current `ConnectionEngine` lease on every Catalog response;
the renderer does not hardcode Disconnected or infer Connected from a kubeconfig.
`catalog_connection_states` reads only the current in-memory lease, without file
discovery, authentication helpers or Kubernetes traffic. Catalog refreshes that
projection while a runtime state is active, using one non-overlapping read every
two seconds while visible and on focus. Local session transitions invalidate
older reads and Catalog responses; failed status reads become Unknown. Connected
uses a success badge and errors use an error badge.
The existing single-workspace lifecycle is unchanged: leaving the workspace
releases its connection. Catalog shows Disconnecting until release completes,
then Disconnected; it never presents a historical successful connection as live.
`tests/catalog-connection-status.spec.ts` and Rust `runtime_status_tests` cover
projection, stale replies, replacement leases, disconnect and persistence isolation.

### Cluster workspace and row activation

Catalog rows activate with a click, Enter, or Space. Cluster rows and pinned
cluster shortcuts open `/cluster/<saved-id>`; no activation creates a Kubernetes
connection. Pin/Remove buttons and other nested controls never activate their row.
The selected cluster is resolved exactly from the loaded catalog, never guessed
from a name or nearby record. Unknown/deleted IDs show a missing-cluster state.

The workspace adds a cluster-specific inset sidebar beside the collapsed main
sidebar, preserving the Astryx section shell. Cluster identity lives in the toolbar
selector, without a duplicate sidebar name header. The sidebar routes
between Cluster, Nodes, Namespaces, Events and the leaf destinations inside
Workloads, Config, Network, Storage, Helm, Access Control and Custom Resources.
Grouped parent rows are disclosure buttons: clicking the label or chevron toggles
the children without changing the page or closing the narrow-window drawer.
Only child activation navigates. Existing parent-section URLs remain available
as legacy destination directories. These are navigable workspace sections
with explicit disconnected or unsupported states where applicable. Nodes and Namespaces
show read-only resource tables after automatic connection. Resource tables, Pod
sessions, Helm inventory, and loopback port-forward sessions are also Rust-owned
workflows described below. Show saved metadata in Cluster actions opens a named
Astryx dialog with saved cluster/context/source/labels, Escape dismissal and focus
return, not an inline workspace panel. These saved fields do not prove connectivity.
The global toolbar owns Refresh and Cluster actions. Cmd/Ctrl+R refreshes the
active Catalog category or registered workspace view, including overview,
resource tables and custom-resource rows. A focused resource inspector captures
the shortcut for its own read and stops propagation, not the table behind it.
Modified, composing and repeated keys must not start duplicate reads. No
fabricated metrics, events, counts, or terminal output.

The desktop workspace sidebar has a right-edge Astryx resize handle. Dragging
reflows the working surface immediately; Arrow Left/Right, Home and End provide
keyboard resizing with a pane-linked separator and visible focus. Width is bounded
to 176 CSS pixels minimum and the smaller of 35% of the viewport or 480 CSS pixels
maximum, preserving working space. The initial width remains 18% capped at 256.
Astryx persists the bounded width under
`astryx-resizable:iceon.cluster-sidebar-width.v1` as a non-sensitive UI preference;
it is shared across clusters and restored after returning from Catalog or reloading.
Shrinking the window clamps and saves the width. The icon-only Catalog rail does
not resize, and the narrow-window drawer remains full-width without a handle.
Only sidebar exit animates width; dragging never waits on a width transition.
This is a user-requested Iceon interaction, not a new verified Freelens workflow.
Rust ownership is unchanged: no new commands, connection work or persistence of
domain data. `tests/sidebar-resize.spec.ts` covers pointer/keyboard bounds,
cancellation/disposal, persistence/storage failures, themes and responsive reflow;
native webview resizing and OS fullscreen behavior require a separate smoke test.

Section identity is scoped to the cluster route through the validated `section`
search field. History back/forward and direct links retain cluster/section scope.
Switching clusters resets the temporary sidebar expansion. On narrow windows the
shared drawer contains Catalog and Cluster navigation; choosing a section closes
it. Back to Catalog remains available without requiring the drawer.

Hash routing works in packaged webviews. Navigation state is separate from the
engine's connection state. On narrow windows Astryx moves navigation into its
drawer; keep focus trapping, Escape dismissal, and focus return intact.
Tables may scroll inside their region, but the window must not overflow sideways.
Long identifiers must not push actions or window controls off-screen.

## Search, loading and detail feedback

The global Cmd/Ctrl+K palette searches saved cluster names, contexts, file labels,
and static cluster menu destinations alongside the existing Catalog pages.
`appSearchSource.ts` indexes normalized metadata once per Catalog snapshot and
matches it against shared menu templates, without a cluster/menu Cartesian index,
network traffic or an input debounce. Results are capped at 60; users can combine
cluster/context and menu words to refine them. Every result resolves an exact saved
cluster ID and validated section, including when cluster names are duplicated.

Search uses the shared Rust-backed `PermissionStore` observations for the current
connected lease. Known denied cluster-scoped destinations are omitted, including
during review renewal; known all-namespace denials retain namespaced menus with
guidance to choose a namespace. Unknown access and other clusters are explicitly
Access unverified, not permission grants. Search never connects clusters merely
to enumerate permissions. Selection rechecks the current cached navigation policy;
the destination and Rust resource services retain their existing authorization.
Open-query results update when metadata or permission observations change. Loading,
failed/retained and empty Catalog states remain visible inside the palette.

This is an Iceon-requested workflow, not a verified Freelens port. The local
`freelens` checkout was unavailable for this change; the prior recorded reference
is v1.10.3-533 at `cd07118f843c4a69f937a319a37a7c12ef2c92d8`.
Rust owners remain CatalogEngine/ConnectionEngine and `connection/permissions.rs`;
no new renderer networking or backend endpoint is introduced. Acceptance:
`tests/app-search.spec.ts` covers combined searches, duplicate identities, keyboard
selection, open-query updates, denial/namespace/session isolation, browser fallback,
both themes at narrow widths, and a bounded 1,000-cluster local-search benchmark.
All fixtures are synthetic and IPC is mocked; they do not verify live RBAC or
native webview performance.

The renderer hook `src/app/cluster/useDebouncedSearch.ts` commits controlled search drafts
after 160 ms of quiet input. Resource reads cancel obsolete work immediately;
retained rows are marked stale and scope-dependent actions disable while the
draft is pending. Catalog and cached Helm/forward filtering instead use
deferred local queries and memoized searchable fields; namespace options filter
their loaded cache locally. Row-menu fingerprints are
memoized by rows/actions, rather than serializing the full inventory on every
keystroke. Resource action/selection scopes use the committed query plus pending
state, so subsequent draft characters do not remount actions or rerender retained
cells. Row-menu transforms keep their identity while validating the current scope
on open/selection; permission fingerprints cover the active row, not every loaded
row. Checkbox enabled-state changes still invalidate the selection plugin, and
handlers reject selection while unavailable. Search drafts survive namespace changes
while inventory readers remount with the new namespace and cancel the old request.

The app uses React directly. The `@tanstack/redact` Vite runtime replacement was
removed after CPU profiling identified synchronous element/owner-stack bookkeeping
in the input path; its synchronous runtime also undermined deferred rendering.
React DevTools stays opt-in via `ICEON_REACT_DEVTOOLS=1`. Helm's local query no longer
re-registers global Refresh on each pending/settled transition, and unchanged filter
results retain their row-array identity. No credentials, network or backend ownership
move into the renderer.

Repeatable latency check: `bun run build`, then
`ICEON_TEST_PRODUCTION=1 bun run test:e2e tests/search-responsiveness.spec.ts -g
"production input latency" --workers=1`. This serves the optimized bundle on port
1422 with trace snapshots disabled so profiling/DOM capture do not skew input frames.
Synthetic IPC fixtures cover 500 Catalog/Helm rows and 100 Pod/Node rows. The measured
run had input-to-next-frame p95 of 15.7–17.3 ms; maximum observed delays were
27 ms (Catalog), 189.6 ms (Helm), 143 ms (Pods), and 142.7 ms (Nodes), including
first-edit setup. These are Chromium fixture measurements, not native-webview,
live-cluster response time or FPS claims. Development render-count and cancellation
tests remain separate from this production timing check.

`TableToolbar.tsx` keeps search compact (at most 18rem) on the right, beside status
and other filters, rather than stretching across the remaining space. Resource and
Helm tables keep namespace context beside the title. A compact information/warning
icon immediately to the right of the namespace selector exposes discovery/cache and
nonblocking resource-access explanations on demand, not a page-level banner.
Routine permission checks run quietly with a neutral "Namespace options" tooltip;
unavailable reviews do not trigger a warning color or repeated explanation. Confirmed
denials and namespace-load errors remain discoverable. Clicking the icon opens an
accessible popover with one concise explanation, manual namespace entry and retry/load
actions. Blocking resource access failures remain in the content area, and permission
checks still disable unavailable mutations. At narrow widths controls wrap without
horizontal page overflow. Namespace
selectors use one built-in loading indicator, with a named announcement rather than
a second visible spinner.

`ContentLoading.tsx` centers a visibly labeled Astryx spinner in the remaining
content region on an initial load or first permission check, leaving filters editable.
Retained content uses a nonblocking progress bar below filters; reduced motion stops
animation. Failed refreshes remove progress without hiding errors or retained stale
rows. `tests/resource-toolbar.spec.ts` covers the Vertical Pod Autoscalers loading,
failure/retry and unverified-access states in both themes and 360–1568-wide layouts.

`ContentError.tsx` centers blocking first-load failures below the top search/filter
controls instead of showing an empty table. Retained data gets a compact error
and stale notice, not a blocking replacement. Offer retry and its shortcut only
when there is a working action. This keeps failure distinct from a successful empty
result; errors wrap at narrow widths in both themes.

Resource inventory tables use Astryx `EmptyState` for successful empty and
filtered-empty results, with a centered heading, scope-aware description, and
working Refresh or Clear filters action. Loading and failed empty snapshots do
not show a success-style empty state; failures retain rows only when rows exist.
`tests/resource-tables.spec.ts` covers StatefulSets in both themes at desktop and
narrow widths, keyboard recovery, namespace preservation, and empty-refresh failure.
Port-forward inventories use the same responsive `EmptyState` layout, offering
Create forward for an empty session and Clear filters for no matches. The create
action only opens the existing form; starting still requires validation and explicit
confirmation. `tests/service-tables.spec.ts` covers keyboard actions, both themes,
narrow layouts, and loading/failed-empty refresh separation.

The compact details header shows the plain resource name and tooltip-equipped,
accessible icon controls. Resource edit/delete, workload and Node scheduling launchers,
and Service forwarding join Pod Logs/Shell/Attach/forward in this header rather than
in the scrolling body. A panel-width-aware More actions menu exposes named choices
when these controls would crowd the title; Refresh, Expand/Restore and Close remain
reachable. Service forwarding uses one launcher with reviewed TCP port selection.
Dialogs and local form submissions retain explicit text labels and existing identity,
permission, dry-run and confirmation checks. Refresh is blocked while an inspector
workflow is open or submitting; inline Pod YAML has Cancel editing to leave it safely.

The body scrolls vertically independently of the table. Shared detail lists use at
most two row-major columns against actual panel content width, falling back to one
below 28em, including enlarged text. Identity, labels, annotations, Pod facts, Helm
facts/history and captured summaries use this layout. YAML, managed-field JSON and
related tables retain their reading width. Managed fields have a named local icon
disclosure with announced expansion and a controlled region; related collection
Retry icons stay beside their own headings and reload only that collection.

Refresh retains same-object details as stale on failure, rather than presenting old
data as current. Pod edit/mutation launchers and submission stay disabled after a
failed read or uncertain mutation until a successful fresh read; failed writes are
not replayed automatically. Logs/Shell/Attach/forward retain their existing scoped,
permission-checked launch policy rather than being reclassified as mutations.
Focused Helm details honor the same Cmd/Ctrl+R shortcut as resource details and have
one header retry, preserving sanitized errors and retained-stale notices.

This is an Iceon-requested presentation refinement, not a new Freelens workflow.
`tests/compact-inspector.spec.ts`, `tests/pane-accessibility.spec.ts`,
`tests/detail-columns.spec.ts`, `tests/pane-integration.spec.ts` and
`tests/helm-details.spec.ts` cover the toolbar, reflow, recovery and focus contracts
with synthetic IPC. Native OS interaction and real-cluster outcomes require separate
verification.

Generic detail/cancel commands in `src-tauri/src/lib.rs` extract the registered
`Arc<ConnectionEngine>` state; the former non-Arc extraction mismatch caused generic
detail invocation failures before Kubernetes requests. Kind-aware RBAC name rules
are shared by inventory projection and details, including `system:Reader` encoded
as one named-object path segment. The state regression checks source signatures,
not actual native invocation. See [resource inspector reference](docs/resource-inspector-reference.md).

Latest validation for these search/theme/auth changes is pending. Historical
suite totals in the workspace reference are not a claim about this revision;
no real credentials or native smoke tests were used for this documentation pass.

## Node drain

The Node inspector exposes a conservative **Drain** action through the existing
inspector toolbar. Rust `connection/node_drain.rs` owns read-only review, a
single-use random plan bound to the Node UID/resourceVersion and authorization
epoch, 120-second plan expiry, guarded cordon and sequential `policy/v1` Pod
evictions. The renderer displays bounded Pod dispositions, requires the exact
Node name before confirmation, and retains the execution outcome until closed.
Opening the dialog never reviews or mutates the cluster by itself.

Drain respects disruption-budget rejection and Pod grace periods. It never uses
kubectl, force deletion, direct Pod DELETE fallback, local-data overrides, automatic
write retries, rollback or automatic uncordon. Mirror and verified DaemonSet Pods
remain; unmanaged/orphaned Pods, unsupported controllers and local or unrecognized
volume sources block this version. A continuation beyond the 100-Pod bound is an
incomplete review, not authorization to drain a partial inventory. Only eligible
evictions require Pod GET plus core `pods/eviction` CREATE access; skipped and
waiting Pods still require reviewed read access but no eviction grant.

Execution revalidates the reviewed identities before writes and observes Pod
removal and final inventory before reporting complete. Stop, disposal, disconnect,
timeout and changed authorization stop future work but cannot undo submitted
writes. Complete is not infrastructure-shutdown certification; skipped Pods remain,
and Kubernetes cannot make the multi-resource workflow atomic. Pending work blocks
app update/restart until it settles. See
[`pane-analysis/18-node-drain-implementation.md`](pane-analysis/18-node-drain-implementation.md)
for the pinned reference, acceptance tests and deliberate force-drain differences.
Privileged Node shell and historical metrics remain separate work.

## `seamless` window contract

`seamless` is a window/layout mode, not an Astryx `variant`. It removes the
default OS title-bar surface and automatically supplies safe-area clearance within
the app shell's toolbar. Pages never add their own title-bar margin or spacer.

**Ownership:** `src-tauri/src/window.rs` owns the `SEAMLESS` flag (currently true),
native construction, and platform metadata. `tauri.conf.json` keeps the main
window's dimensions and `create: false` so Rust creates it exactly once.
The initialization script publishes frozen `__ICEON_WINDOW_LAYOUT__` metadata
before React mounts. This metadata is a rendering hint, not a security boundary.
`MainLayout` passes it to the app-owned `WindowFrame` for device insets;
`AppToolbar` passes it to `WindowToolbar` for native-control clearance and actions.

| Environment | Native behavior | Automatically reserved area |
| --- | --- | --- |
| macOS | Overlay title bar with hidden title, retaining real traffic lights and OS behavior. | Traffic-light exclusion region at physical left of the toolbar. |
| Windows/Linux | Decorations disabled; Tauri handles minimize, maximize/restore, close, and drag. | Accessible window controls at the end of the same toolbar, outside drag regions. |
| Browser preview | No native configuration or native API calls. | Device safe-area insets only; no imitation desktop title bar. |
| `seamless: false` | Normal native decorations. | Device safe-area insets only; no duplicate application chrome. |

The toolbar uses spacing tokens and ordinary layout flow, with one reserved height
of spacing-10 + spacing-1. The macOS exclusion width is twice spacing-10. These
are centralized layout budgets, not native measurements; validate them when
changing density, platform integration, or window behavior. Physical device
insets use `env(safe-area-inset-*)`, with `viewport-fit=cover` enabled.
Browser safe-area values alone do not describe macOS traffic-light bounds.

The frame fills the dynamic viewport and gives the shell only the remaining
height. Shell pages scroll internally. The toolbar's native-control clearance remains in
fullscreen too, providing a stable drag/control surface; do not add a second
fullscreen offset. Interactive controls must never be drag targets. Failed
window actions produce an accessible error instead of an unhandled rejection.
Only the required window-control capabilities are granted to the main window.

On macOS, the native **Iceon > Preferences…** menu item (⌘,) focuses the main
window and opens the existing Preferences dialog through a main-webview-scoped
Tauri event. Rust owns menu construction and preserves the default macOS menu
actions. The renderer releases its listener on disposal; browser preview never
subscribes to native menu events. The toolbar button and keyboard shortcut remain
available, and terminal theme matching is labeled **Match app theme**.

## Engine boundary

- **Rust/Tauri:** native lifecycle, Kubernetes clients and watches, kubeconfig,
  credentials, filesystem access, durable domain storage, proxies, subprocesses,
  terminals, logs, forwards, metrics, and Helm integration.
- **React/TypeScript webview:** presentation, routes, focus, sorting/filter UI,
  temporary form state, and typed requests/subscriptions to Rust.
- **Node/Bun:** frontend dependencies, Vite/build tooling, and development tests
  only. No Node server, Electron main process, runtime JS backend, or bundled
  Node/Bun sidecar. The packaged frontend is static assets in the system webview.

Keep IPC typed and scoped to a cluster/context/namespace. Rust validates inputs
and authorization regardless of UI state. Streams have bounded buffers and an
explicit cancellation/disposal path. Never send kubeconfig contents, tokens, or
private keys to the renderer merely to display connection status.

The old `iceon.pinned-pages.v1` UI preference is no longer read or written;
existing category pins do not migrate into item pins. Pinned cluster/link identities
are stored through Rust-managed domain storage. localStorage is reserved for non-sensitive UI preferences, not
credentials or cluster domain state.

### Cluster destination pins

Each static cluster-sidebar row has a separate Pin/Unpin action. Pinning does
not navigate, toggle a disclosure, or dismiss the narrow-window drawer. A saved
shortcut opens the exact cluster ID and section; a group shortcut opens its
existing destination directory. Resource access still follows the normal
workspace connection and permission checks. Dynamic custom-resource instances
and Catalog category pages are not pinnable.

Unpinned actions appear on row hover or keyboard focus; pinned actions remain
visible. Devices without hover keep actions visible. Group pins sit before the
expand/collapse chevron. Config children use the same controls as other static
children. The Cluster/overview action shares the Catalog cluster pin rather than
creating a second destination shortcut. Previously saved overview section pins
are validated then folded into that cluster pin on load; the next save persists
the canonical representation, and unpinning cannot resurrect the old shortcut.

Shortcuts use the existing section icon (Pods uses a cube) and the owning
cluster's existing color renderer. Names include both cluster and destination
to distinguish identical sections in different clusters. Shortcuts retain
accessible names and tooltips. Right-click, Context Menu or Shift+F10 opens a
Rust-created native menu with Unpin and Move up/down; the cluster sidebar retains
its Pin/Unpin actions. There is no renderer imitation of a native menu. Browser
preview reports that native menus require the desktop app.

Rust `CatalogEngine::set_section_pin` owns persistence and validates an exact
static section allowlist plus an existing cluster ID. SQLite schema 4 retains
`pinned_sections` JSON and adds ordered `pin_order` rows, upgrading schema 1/2/3 without changing
catalog revision, item pins, colors or provenance. Reimport retains section pins
only for surviving cluster identities. Removing a cluster removes its pins.
Repeated writes are idempotent; failed saves retain previous data and display
the existing catalog error. Pins never read kubeconfig files or persist session,
namespace, resource-selection or credential state.

The Catalog snapshot's `pinnedItems` is the authoritative ordering of
cluster and section targets. Internal drag/drop reorders this list;
Alt+Arrow Up/Down and native Move up/down provide keyboard alternatives. Reorder
is a Rust-validated exact permutation, not an arbitrary list of routes. New pins
append; unpin, removal and stale-import reconciliation remove only affected
targets. Failed saves keep the last saved ordering. Dragging never activates the
shortcut, accepts external pin data, or reads dropped files.
Tauri's native file-drop handler is enabled for kubeconfig selection. Native pin
reordering uses mouse/pen pointer events instead of HTML drag events (which
WebView2's native handler intercepts); browser preview retains HTML drag events.
Keyboard movement is unchanged. No filesystem/menu/opener capability is added.

Native menu requests are bounded to approved window labels and the saved pin,
use one-shot random native IDs, expire after 30 seconds, and are canceled on
navigation, pin-list changes, disposal and window shutdown. Late replies cannot
unpin another target or mutate a disposed route. Tests cover these IPC contracts
with synthetic metadata in `tests/pinned-items.spec.ts`, Rust menu lifecycle
tests in `pin_menu.rs`, and migration/reorder fixtures in `catalog/pin_order_tests.rs`.

This is the requested Iceon workflow, not a newly verified Freelens port. The
previous catalog reference is Freelens v1.10.3-533 at
`cd07118f843c4a69f937a319a37a7c12ef2c92d8`; the local `freelens` checkout was
unavailable during implementation, so no upstream section-pinning parity is
claimed. Acceptance fixtures: `catalog/section_pin_tests.rs` covers migrations,
restart, reconciliation, bounds and corrupt records; `tests/section-pins.spec.ts`
covers exact routes, icons/colors, keyboard actions, save failure and narrow
drawer behavior with synthetic metadata and mocked IPC only.

### First catalog engine slice

`src-tauri/src/catalog.rs` and `catalog/storage.rs` own `catalog.sqlite3` in Tauri's
app-data directory. Commands in `lib.rs` delegate to serialized blocking engine
operations. Normalized metadata and cluster tables are updated in
SQLite transactions, with a cross-process lock and bounded storage. Unix
directories/files are private; Windows inherits OS ACLs. Corrupt or unsupported
stores fail closed and are never silently reset. The original `catalog.json`
migrates once, preserving IDs/pins/revision and adding colors; its original bytes
and a non-overwriting `catalog.json.backup` remain. SQLite is authoritative afterward.

- `catalog_list` returns a revision, metadata-only clusters, and a safe
  nullable `discoveryError` code if the built-in default-file scan failed.
- Add config offers **Choose file** through Rust `catalog_choose_kubeconfig` in
  `catalog_picker.rs`, using a native dialog parented to the main window. It returns
  only a selected absolute path or cancellation; it does not read/import/connect.
  The editable path remains available. Cancel leaves it unchanged, and late dialog
  results cannot update a closed/reopened form. `catalog_import_kubeconfig` remains
  the validator, including the 1 MiB limit; do not repeat that limit as routine form
  copy. Native dialog behavior has not been smoke-tested.
- Add config keeps Choose File beside the path and platform guidance collapsed
  behind an optional disclosure. The file area accepts one native dropped path
  only while idle and under the pointer. Physical coordinates are scaled to CSS
  coordinates; drops never read contents in React or automatically import.
  Multiple files, relative paths and URLs are rejected without replacing the path.
  Listeners are disposed on close, pending picker/import and unmount, including
  late registrations. Rust still validates regular-file/size/symlink constraints.
  `tools/tests/fileDrop.test.ts` covers path bounds and listener lifetime;
  `tools/tests/catalogForms.test.tsx` mounts real Astryx components under React
  StrictMode for open/help/pending/cancel/reopen hook-order regression coverage.
  Native drop/zoom/window behavior and pointer reorder need OS smoke tests.
- `catalog_import_kubeconfig` accepts an explicit absolute regular-file path.
  Default-file discovery is separate from explicit imports; there is no `KUBECONFIG`
  environment or directory scan. Files are capped at
  1 MiB; only valid named contexts and cluster references enter the catalog.
  Imports reconcile that source, preserve surviving pins/colors, and remove stale contexts.
  No credentials, server addresses, certificates, or auth/exec configuration are
  retained or emitted. The store keeps the canonical source path for identity;
  a separate persisted alias records the selected path for the file provenance
  label, abbreviating the default path to `~/.kube/config`.
  Symbolic links are supported for explicit imports and default discovery,
  including relative/chained targets and paths through linked directories.
  Resolve the target before opening, then require a readable regular file within
  the same size bound. Broken/cyclic links, directories, and special files are
  rejected without changing previous records. Link resolution never executes
  auth plugins or reads any additional kubeconfig files.
  Canonical identity deduplicates alias/direct imports and preserves pins/colors.
  The last successful import selects the displayed alias for a canonical source.
  Retargeting an alias reconciles its old rows; separately selected old targets
  remain. Alias provenance is persisted in SQLite schema 2; schema-1 catalogs
  upgrade transactionally without losing IDs, revision, preferences, pins, or colors.
  Database, lock, sidecar, and legacy-storage symlinks remain rejected. File metadata
  checks are best-effort replacement detection, not a hostile-filesystem race guarantee.
- Default-file discovery is built in, with no setting or routine information UI.
  Every catalog load and Refresh imports metadata from `~/.kube/config`, including
  after restart and for catalogs that previously had discovery disabled. Legacy
  SQLite preference data is retained for compatibility but no longer controls reads;
  there is no `catalog_set_discovery` command or `autoDiscover` response field.
  The home path is supplied by Rust, not the renderer or shell expansion. This is
  not a background file watcher. An absent default file with no previously discovered
  rows is normal and silent. Broken/unreadable/invalid files or disappearance of a
  previously imported default source preserve previous rows and show an actionable
  warning. Mutations do not rescan. Removing an auto-discovered record is local only:
  it returns on the next load/Refresh if still in the file; the dialog explains this.
- `catalog_set_pin` accepts an existing entry ID, never a category route.
- `catalog_remove` removes local metadata and its pin after UI confirmation;
  it does not delete source files or change clusters.
SQLite schema 5 removes the retired `web_links` table and its pins transactionally,
without reading obsolete link payloads. Cluster IDs, colors, pin order, section
pins, preferences, provenance, and revision survive. Legacy JSON imports ignore
retired links while preserving cluster metadata and the original/backup files.
No add/open web-link commands or renderer opener permissions remain. This is a
user-requested simplification, not a new upstream workflow. Migration acceptance
is covered by Rust `catalog/pin_order_tests.rs` and `catalog/tests.rs`; browser
coverage is in `tests/catalog.spec.ts`, `catalog-activation.spec.ts`,
`pinned-items.spec.ts`, and `app-search.spec.ts`.

The catalog is bounded to 1,000 entries, 4 MiB serialized metadata, and a 16 MiB
SQLite database. Rust generates a random `#RRGGBB` color once for each new entry
using OS randomness and persists it with pins; imports/reloads never reroll it.
Stable IDs deduplicate contexts by source/context and links by normalized URL. Revisions prevent stale
UI responses replacing newer ones. `CatalogProvider` shares snapshots with the
table/sidebar, serializes UI actions, and ignores disposed requests. Errors use
fixed codes mapped to safe messages, never parser output or credential excerpts.
An in-flight local write completes even if its initiating view is disposed; no
watch or background discovery session is started. The UI preserves loaded data
on refresh failure. Browser preview clearly disables engine-backed actions.

### Cluster connection and live resource foundation

`src-tauri/src/connection.rs` owns one main-workspace session lease. Opening a saved
cluster creates a random lease from catalog metadata only. It does not read the
kubeconfig or contact an API. `cluster_connect` requires that exact cluster/lease,
loads the saved canonical source and selected context, then verifies `/version`.
React keeps only typed public session/resource DTOs. Connection state is ephemeral,
not persisted into Catalog's metadata snapshot.

- `cluster_session_open`, `cluster_connect`, `cluster_connection_status`, and
  `cluster_disconnect` scope every action to a saved ID and lease. Workspace change,
  Cancel, Disconnect, and app/window teardown cancel owned work. Late completions
  cannot publish into a replacement lease. Repeated cleanup is idempotent.
- `connection/config.rs` reads the canonical kubeconfig and referenced credential
  files with the existing regular-file/replacement checks and a 1 MiB per-file bound.
  Relative references resolve beside the canonical source. A retargeted provenance
  alias fails with `sourceChanged`. Only the selected context/cluster/user enters
  client setup; `KUBECONFIG`, in-cluster fallback, debug overrides, and scans are not used.
- Connections support verified HTTPS with static token/basic/client-certificate
  credentials, bounded non-interactive exec credential helpers, and supported cached
  auth-provider credentials through `connection/auth.rs`. Explicit proxies, HTTP,
  and insecure TLS skip settings fail closed with fixed codes. Configured exec and
  token-file sources support request-time renewal through a shared Rust lease;
  cached legacy provider/static credentials do not auto-refresh. See
  [credential renewal](docs/credential-renewal.md) for replay and lifecycle limits.
  Credential helpers are Rust-owned, not Pod terminals. Inherited proxy routing is not enabled.
  On macOS, a helper without an explicit kubeconfig PATH receives a child-only,
  bounded PATH containing inherited absolute directories and fixed Homebrew/Google
  Cloud SDK installation locations. If spawning still reports NotFound, bounded
  system PATH hints and literal Google SDK source declarations can resolve a
  custom installation. Literal fish SDK source/PATH declarations and saved
  `fish_user_paths` are recognized in bounded user configuration files as well.
  No login shell, shell-file evaluation, recursive search,
  automatic installation or global environment modification is used.
  Explicit `exec.env.PATH` remains unchanged. See `connection/auth_path.rs` and
  [credential renewal](docs/credential-renewal.md) for bounds and fixture coverage.
- `connection/transport.rs` uses `kube` 4.2.0 with rustls. It sends only allowlisted
  GETs for version and supported resource inventories. Pod subresources and confirmed
  mutations have separate scoped engine services, not arbitrary renderer URLs.
  Requests have a 15-second deadline,
  four-operation admission bound, cancellation inside the client's buffered service,
  and body limits before JSON parsing (64 KiB version, ordinarily 2 MiB resource
  response; CRD-only limit-1 inventory reads permit 16 MiB as documented below).
  API error bodies and underlying credential/TLS/transport messages never reach IPC.
- `cluster_resources` fetches pages of at most 100 allowlisted resources with
  per-kind safe projection and validated continuation. Nodes retains its own
  facts/optional-metrics table. Continuation is reported as `truncated`, never
  a total. Monotonic per-read IDs bind `cluster_resources_cancel` to the exact read;
  cancellation-before-registration and out-of-order completions fail closed.
  Tables show loading, empty, denied/error, partial, and stale retained
  rows on refresh failure. Connected means the version probe succeeded, not that all
  resource permissions exist or that background health monitoring is running.

Behavior reference remains local Freelens commit
`cd07118f843c4a69f937a319a37a7c12ef2c92d8`:
`common/cluster/cluster.ts` separates connection/accessibility state and
`common/cluster/list-namespaces.injectable.ts` lists namespace names. Iceon keeps
those interactions but uses Rust instead of Electron/Node; no code was copied.
Acceptance: `connection/tests.rs`, `connection/lifecycle_tests.rs`, and
`tests/cluster-connection.spec.ts` cover exact scope, bounded responses, independent
RBAC, cancelled/superseded work, redaction, retry, keyboard use, and narrow layouts.
Tests use synthetic kubeconfigs, an in-memory client service, and mocked IPC only;
they do not prove real-cluster TLS/authentication or native OS integration.

### Automatic foreground inventory updates

`src/app/cluster/useAutoRefresh.ts` schedules typed IPC reads, not renderer cluster
networking. First-page resource inventories (`ResourceTablePage.tsx` and
`ClusterResources.tsx`, including Nodes/Namespaces) repeat 10 seconds after
completion; `HelmTablePage.tsx` and `WorkloadsOverview.tsx` use 30 seconds. Rust
`connection.rs`, `connection/transport.rs`, `resources.rs` and Helm services retain
their existing request bounds, validation and session/read cancellation. Overview
still uses its sequential five-page/500-item-per-kind budget, not an unbounded LIST.

Scheduling requires a visible, focused, online document. Focused input/select/
textarea/contenteditable controls, visible dialogs/alert dialogs, menus/listboxes
and expanded popups block automatic refresh. Pending filters and active Pod Live
sync also pause ordinary resource polling; Helm installation pauses Helm polling.
Focus, visibility and online events attempt catch-up subject to the same guards.
No automatic read overlaps its pending read. Transient `timeout`, `network`, `busy`
and `connectionFailed` errors use exponential backoff capped at 60 seconds; other
errors stop automatic retries until scope changes or an explicit successful retry.
Disposal removes timers/listeners and owning effects cancel their exact Rust reads.

Continuation pages are manual snapshots: automatic refresh neither advances pages
nor jumps back to page one. Toolbar Refresh remains explicit. Resource/Helm
background delivery waits while editing/popups block it and preserves equivalent
inventory snapshots when only request IDs/fetch times change. Failed updates retain
rows with actionable stale/error feedback; fetched timestamps are no longer rendered.
The existing opt-in Pod LIST/WATCH remains separate and does not imply all-kind
watch coverage or full Freelens parity. Resource detail/YAML reads are still manual.

Behavior reference: previously agent-verified local Freelens commit
`cd07118f843c4a69f937a319a37a7c12ef2c92d8`,
`packages/core/src/common/k8s-api/kube-object.store.ts` (`subscribe` workflow).
Its subscription behavior is the reference, not an assertion that bounded polling
duplicates upstream watches. No upstream source was copied. Acceptance sources:
`tests/auto-refresh.spec.ts` (completion scheduling, overlap, backoff, terminal stop,
editing/dialog pause, visibility/online return and teardown) and
`tests/resource-tables.spec.ts` (quiet Pod LIST, delayed delivery and manual
continuation). These fixture obligations are not a current passing-suite claim.

### Application updates and distribution

The `0.0.3` route targets Apple Silicon macOS and Windows 32-bit x86. Rust
`updates.rs` owns bounded GitHub `latest.json` checks, newer stable-version
selection, package download and the official Tauri signature/installation path.
The embedded minisign public key and authenticated package version are required;
there is no unsigned fallback. This artifact signature is separate from OS trust:
macOS remains ad-hoc signed without Developer ID/notarization, and Windows has
no Authenticode certificate. Existing 0.0.1/0.0.2 installations have no updater
trust key and need a one-time manual 0.0.3 installation.

After bootstrap, verified updates download automatically, but installation and
restart require the toolbar's **Restart to Update** action. This visible secondary
button uses existing Astryx styling so the pending action is discoverable without
interrupting work. Readiness is not consent, and clearing a blocker or timing out
requires another click rather than an automatic retry.
Every admitted native window must acknowledge the current preparation ID
after freezing interaction. `WindowGate` blocks new windows during preparation
and restart. Dirty editors, dialogs, mutations, terminals and forwards defer
installation; the backend closes admission and drains session work before install.
The UI reports `preparing` and `restarting` without editing or saving user drafts.
See [automatic updates](docs/automatic-updates.md) and
[release instructions](docs/releases.md) for trust, limits and installation warnings.
CI packaging, Windows x86 compilation and native update/install smoke verification
remain release gates, not results established by workflow configuration.

### Resource tables and namespace scope

Resource tables are normal workspace destinations and load when their section is
entered. There is no Resource tables toggle or stored preference. Fixed parent
pages use `DestinationGrid.tsx`: descriptive, keyboard-operable card anchors to
existing destinations, not a resource table with a View column, invented aggregate
counts or fan-out requests. Resource inventories stay compact tables. Nodes retains its independent
table behavior.

`src/app/cluster/NamespaceScope.tsx` is the shared selector for every connected-lease
page that needs namespace scope. Its ephemeral Zustand store in `namespaceStore.ts`
is owned by the connected cluster/session, not by individual page subscriptions.
It lazy-loads namespace names once when the first namespaced consumer mounts,
using the Rust `cluster_resources` path for the
`namespaces` resource. Each Rust page contains at most 100 entries; the shared
loader stops at 1,000 names or 10 pages. These are safety bounds, not a claim about
the cluster's namespace total. The selector exposes the loaded names, reports
partial results honestly when the bounded list is incomplete, and shows loading,
empty, failed/retry, and no-search-match states. "All namespaces" remains available
for inventory scope even when namespace discovery fails; the forward form requires
a specific namespace. Search filters loaded selector options locally. There is no
free-text namespace entry or Apply step: choosing a namespace takes effect immediately.

Successful, empty, partial and failed selector results are reused across page
navigation, including visits to pages without namespace filters. Consumers also
share an in-flight discovery; leaving one page does not cancel it. Automatic table
refresh, permission renewal with the same authorization epoch, search, pagination
and filter changes never refetch the selector list. Explicit workspace-toolbar
Refresh or Cmd/Ctrl+R invalidates the namespace cache alongside the current page;
an explicit namespace retry is also available after failure or unverified access.
If no selector is mounted, refresh invalidates lazily for the next consumer.
Disconnect/session replacement disposes the store and cancels its exact pending
read. Permission denial or a changed authorization epoch clears cached names and
invalidates old results; normal permission/backend checks still gate every read.
No namespace data is persisted to localStorage. Generation checks reject results
from before refresh, disposal or authorization invalidation. Lifecycle coverage is
in `tests/namespace-store.spec.ts` and `tests/namespace-selector.spec.ts`.

Namespace selection immediately scopes the resource read in Rust, resets its
continuation, and disposes the previous read. A changed namespace also disposes the
old Pod LIST/WATCH sync before the replacement is started, so late results cannot
cross scopes. The same selector and immediate-selection contract is used by the
Helm releases page and the port-forward form. Kubernetes Search/Status filters scan
continuation pages in Rust within explicit bounds (10 pages, 1,000 scanned items,
5 seconds, 100 returned matches), retaining the next unscanned continuation when partial.
The port-forward table's existing namespace
filter still filters loaded local sessions; it does not change to a cluster-resource
read or a namespace-discovery request.

Behavior-only reference: local `/Volumes/Data/Kerjaa/freelens`, pinned commit
`088d08513c8c149365dd802c588b9b0b90c6293e`, especially
`packages/core/src/renderer/components/namespace-select-filter/`. No upstream code
was copied. `connection.rs`, `connection/transport.rs`, and `connection/resources.rs`
own the bounded namespace reads, continuation validation, and lease/read scope;
`NamespaceScope.tsx` owns presentation and typed IPC adaptation. Acceptance covers
automatic lazy loading, All/search/selection, partial and retry states, immediate
scope changes, stale-result rejection, and reuse by Helm releases and the
port-forward form in `tests/namespace-selector.spec.ts`.

Behavior-only reference: local `/Volumes/Data/Kerjaa/freelens`, commit
`088d08513c8c149365dd802c588b9b0b90c6293e`. No upstream code was copied.
`connection.rs`, `connection/transport.rs`, and `connection/resources.rs` own
allowlisted GETs, namespace/pagination validation, bounded safe projections, and
session/read cancellation. Kubernetes filtering now belongs to Rust; Helm and local
forward inventories retain loaded-row filtering.
Namespace selection immediately changes the Rust request scope and resets continuation.
Search and Status are bounded server-side scans; CRD Definitions also has a Group filter.
Continuation pages are bounded to 100 items and never presented as a total.

| Source under `packages/core/src/renderer/components/` | Expected interaction / UI limits | Acceptance |
| --- | --- | --- |
| `nodes/` | Independent Nodes table and bounded Rust search; safe node facts and optional metrics remain available independently; `connection/transport/nodes.rs`, `connection/metrics.rs` | `src-tauri/src/connection/node_tests.rs`; `tests/nodes-table.spec.ts` |
| `workloads-{pods,deployments,daemonsets,statefulsets,replicasets,replication-controllers,jobs,cronjobs}/` | Per-kind columns, namespace scope, bounded search/status; distinguish current/desired replicas and CronJob active references without exposing payloads | `src-tauri/src/connection/resource_tests.rs`; `tests/resource-tables.spec.ts` |
| `config-{maps,secrets,resource-quotas,limit-ranges,horizontal-pod-autoscalers,vertical-pod-autoscalers,pod-disruption-budgets,priority-classes,runtime-classes,leases,mutating-webhook-configurations,validating-webhook-configurations,validating-admission-policies,validating-admission-policy-bindings}/` | Safe spec/count columns; Secrets/ConfigMaps inventory remains metadata-only, with no data or key names; creation uses explicit YAML/dry-run/confirmation | `src-tauri/src/connection/resource_tests.rs`; `tests/resource-tables.spec.ts` |
| `network-{services,endpoints,endpoint-slices,ingresses,ingress-classes,policies}/`, `storage-{volume-claims,volumes,classes}/` | Resource tables and filters; counts are labeled as counts rather than addresses; missing values remain N/A | `src-tauri/src/connection/resource_tests.rs`; `tests/resource-tables.spec.ts` |
| `namespaces/route.tsx`, `events/events.tsx`, `user-management/*/view.tsx`, `pod-security-policies/pod-security-policies.tsx` | Namespace labels and role subjects are counts only; event messages are not projected; creation requires dry-run/confirmation; removed PSP API returns unavailable, never empty | `src-tauri/src/connection/resource_tests.rs`; `tests/resource-tables.spec.ts` |
| `custom-resource-definitions/view.tsx` | Definition group/version/scope columns and bounded Rust Group filter; no arbitrary custom-resource URL or payload fetching | `src-tauri/src/connection/resource_tests.rs`; `tests/resource-tables.spec.ts` |
| `helm-charts/{helm-charts,helm-chart-details}.tsx`, `helm-releases/releases.tsx`, `network-port-forwards/` | Rust-owned Helm chart/release tables, explicit installation, and a Rust-owned port-forward session table with start/stop | `tests/service-tables.spec.ts`; `src-tauri/src/connection/helm_tests.rs`, `pod_runtime_tests.rs` |

Replacement views retain loading, empty, filtered-empty, failed, stale, partial,
disconnected, and unsupported-API states. Kubernetes scans are explicitly bounded;
metadata count columns intentionally differ from Freelens payload-rich views.
Acceptance uses mocked IPC and synthetic Kubernetes objects only. It does not
establish real-cluster authentication, native window behavior, or feature parity.

### Resource checkboxes and confirmed deletion

Behavior-only reference: `/Users/user1/Kerjaa/freelens`, commit
`cd07118f843c4a69f937a319a37a7c12ef2c92d8`, inspected 2026-10-26:
`packages/core/src/renderer/components/item-object-list/content.tsx` (row checkboxes,
select-all of enabled visible items, selected-item removal confirmation),
`packages/core/src/common/item.store.ts` (ID-based selection),
`packages/core/src/renderer/components/add-remove-buttons/add-remove-buttons.tsx`
(adjacent remove/add actions), and `packages/core/src/common/k8s-api/kube-object.store.ts`
(selected resource removal). No upstream source was copied or adapted.

`ResourceSelection.tsx` uses Astryx controlled selection with accessible row names,
a mixed-state header checkbox, a count and Clear selection in the first data
heading (after the checkbox). Other heading labels become visually hidden while
selected, retaining accessible labels and column sizing; focused heading controls
become visible. Selection is confined
to the currently displayed Kubernetes page and resets on filter, page, namespace,
kind or session changes. Refresh/live updates retain only visible matching UIDs;
a replacement with the same name is not selected. Rows lacking UID/resourceVersion
remain readable but cannot be selected. Inventory DTOs project only bounded identity
metadata and allowlisted facts, never whole objects or Secret contents. Event
messages are bounded projected text, not guaranteed secret-free content.

`ResourceActions.tsx` groups Delete immediately left of Create at the bottom right,
outside the scrolling viewport and above the Pod dock. Delete is disabled without
selection or while inventory is loading/failed. Confirmation captures exact targets
and shows cluster, context and namespace/name identities. There is no destructive
action on checkbox activation or dialog open. Unlike Freelens's parallel Promise.all
removal, Iceon submits at most one mutation at a time, at most the 100 displayed
resources, and retains per-item success/failure results. Accepted targets are
deselected as each response arrives; failed targets remain selected if still visible.
Stopping a partially completed batch retains those known results alongside the
active request's uncertainty. Refresh or a non-syncing live state invalidates an
open confirmation and stops queued submissions. There is no automatic
retry or optimistic claim that accepted resources have disappeared.

Rust owner: `connection/delete.rs` behind `cluster_resource_delete` and
`cluster_resource_delete_cancel`. Each single-resource operation validates session,
kind, namespace/name, UID and version; performs a bounded preflight GET; then sends
DELETE to an allowlisted path with UID/resourceVersion preconditions and Background
propagation. Normal grace periods/finalizers remain intact; there is no collection
delete, force deletion, renderer URL, persisted selection, or raw server diagnostics.
Delete has an independent per-kind cancellation lane and monotonic request IDs.
Stop/Escape/view disposal cancels exact pending work and prevents queued submissions;
submitted deletes cannot be rolled back. Responses distinguish deletion accepted
from completed deletion, and errors/cancellation preserve outcome uncertainty.

States: unselected, partially selected, all displayed rows selected, disabled/stale,
confirmation, submitting, partial failure, accepted, canceled/uncertain, and disposed.
Acceptance: all 41 Kubernetes kinds, keyboard/checkbox isolation, mixed-state header,
scope reset, UID replacement, exact IPC identity, sequential deletes, partial errors,
response validation, duplicate guard, cancellation/late-result isolation, and
1440/390-width FAB placement with/without Pod dock in `tests/resource-delete.spec.ts`;
precondition/path/response/cancellation tests in `connection/delete_tests.rs`;
identity redaction in `tests/resource-identity.spec.ts` and `resources.rs` unit tests.
Helm releases and local forward sessions are not generic Kubernetes deletion targets;
their uninstall/stop operations are separate workflows.

### Helm and forward table services

Creation actions are bottom-right icon buttons: Kubernetes tables open YAML
creation, both Helm pages open chart installation, and Port Forwarding opens its
namespace/Pod/ports dialog. Dialog open alone sends no mutation. Refresh remains
in the top-row Cluster actions menu; starting a forward resolves the exact Pod UID
before submitting through Rust. Narrow-window bounds, keyboard focus return,
validation, failed-start retry and late-start ownership are covered in
`tests/service-tables.spec.ts`; Helm dry-run/confirmation and cancellation are
covered in `tests/helm-install.spec.ts` and `connection/helm_install_tests.rs`.

`connection/helm.rs` invokes an installed Helm executable from Rust with fixed
arguments, not a shell command. Charts uses `helm search repo -o json` against the
local repository cache; it does not refresh repositories or install charts.
Releases uses `helm list --all` with a validated namespace or all namespaces and a
temporary, selected-context kubeconfig prepared by Rust. Credentials and raw
subprocess stderr never enter the renderer. Rust bounds each output pipe to 2 MiB,
projects at most 500 rows, and supervises the process group/job with cancellation,
deadline, kill/reap, and temporary-file cleanup. Read IDs reject superseded work.
Missing Helm, failure, malformed output, loading, legitimate empty, filtered-empty,
partial, and retained stale inventory are distinct states. Helm's own list limit can
make releases partial even below the display cap. Installation is a separate explicit
Rust-owned dry-run/confirmation flow, not a side effect of reading either table.

The forward table is an inventory of this workspace's Rust-owned sessions, not an
API resource list. It can start a forward after resolving the exact Pod UID, filter
loaded sessions locally, and stop/remove records through typed IPC. Table-started
forwards survive section navigation and stop with the workspace session; tab-started
forwards additionally stop when their owning Pod tab closes. Neither workflow opens
an external browser or exposes a non-loopback listener.

### Pod tables, details, and sessions

The Pod table is a bounded Rust projection of a Kubernetes `LIST`, with an optional
Rust-owned `WATCH` sync on the first page. Its **Enable/Pause live sync** action in
the top-row Cluster actions menu remains opt-in;
it controls Pod LIST/WATCH, not access to resource tables. The engine validates the object kind,
namespace, identity, resource version, continuation, and typed watch event before
projecting safe row fields. Typed snapshots carry a monotonic IPC sequence; raw
watch objects never reach the renderer. It keeps a sorted inventory capped at 100
items, relists
on an expired or incomplete watch, and reports partial data rather than presenting a
total. Namespace changes, continuation changes, refresh, disabling Live sync, workspace
changes, and disconnect cancel the exact owned read or sync. Search/status filters
leave live mode and use the bounded Rust continuation scan described above.
`PodSyncRuntime::start` spawns tracked tasks on the explicit Tauri app runtime,
not an assumed thread-local Tokio runtime. The ordinary-thread regression
`sync_start_from_native_command_thread_uses_app_runtime_and_drains` covers this
native command-thread boundary; browser repeat-toggle/held-read tests cover UI
cleanup, not a native OS crash reproduction.

Selecting a Pod name or row action opens a cluster/session-scoped reusable bottom
dock. Details fetches the Pod and its related events through Rust, and retains stale details when a
reload fails. The dock reuses a details/edit tab for the same Pod identity, supports
logs, shell, attach, and port-forward tabs, and provides keyboard tab selection,
close, minimize, maximize, and resize. Section navigation, switching tabs, and minimizing
keep the dock and its sessions mounted; minimizing is not
disconnecting. Dock and tab disposal closes owned IPC handles; session replacement
and disconnect cancel all child work. A closed or
unmounted tab cannot accept a late stream or detail result.

The dock's **New tab** (+) menu stays available with no tabs and alongside open
or minimized sessions. **New Terminal** asks for a namespace and Pod name, using
the active Pod as its initial target; it opens another existing Rust-owned Pod
shell, not a host terminal. Pod get/exec access checks, container selection,
bounded streams and cleanup remain owned by `connection/pod_runtime.rs`.
**Create Resource** chooses one of the allowlisted kinds and reuses
`ResourceCreate.tsx` for namespace-specific access, YAML, server dry-run and
explicit confirmation through Rust `connection/create.rs`. It does not submit
anything merely by opening the menu or choosing a kind. Session teardown removes
dialogs and cancels their outstanding requests. The compact upward-opening menu
uses existing Astryx tokens and action icons to keep working space available.
Behavior reference: local Freelens `v1.10.3-533-gcd07118f`, commit
`cd07118f843c4a69f937a319a37a7c12ef2c92d8`,
`packages/core/src/renderer/components/dock/dock.tsx:162-180`; no source copied.
Acceptance: dock-action cases in `tests/pod-workspace.spec.ts` and
`tests/resource-create.spec.ts` cover keyboard dismissal/focus, empty/minimized
docks, denied exec, scoped creation/confirmation, stream cleanup and narrow width.
These are mocked IPC checks, not real-cluster or native terminal verification.

Pod details expose a deliberately reduced YAML view. YAML editing is protected by
Rust: the submitted document must preserve immutable identity and resource-version
fields, keep the known container identities and shape, stay within the request bound,
and can change only fields included in the reduced view. Omitted annotations,
environment, commands, volumes, and credentials are preserved in the live object,
not sent as editable payloads or overwritten with redaction markers. Kubernetes may
still reject immutable spec changes. Delete, force-delete,
finalize, and replacement require explicit confirmation and exact Pod UID/resource
version checks. Generic resource creation/deletion is separately bounded to the
41 allowlisted kinds described above, not a mutation or feature-parity guarantee.

Logs use the Kubernetes API log stream through Rust and publish bounded ring-buffer
snapshots over typed IPC. Tail, previous-container, and timestamp choices are scoped
to the Pod/container. Rust retains a 256 KiB UTF-8-safe ring per stream and drains
at most 64 KiB per read; the renderer bounds its retained text to 512 KiB. Search,
pause-follow, wrapping, clear, and downloads operate on buffered output, not an
unbounded server log export. Earlier output is explicitly marked when the ring or renderer
buffer drops bytes. Shell and attach use Kubernetes exec/attach WebSockets through
the Rust client, with bounded input/output, terminal resize, and cancellation. They
never invoke a host shell, `kubectl`, or a renderer-side process. Stream handles are
owned by the dock/session and are closed on tab close, view disposal, replacement,
disconnect, and app teardown.

Port forwards use Kubernetes port-forward through Rust and bind only to loopback
(`127.0.0.1`). The engine resolves and verifies the Pod UID before starting, owns
the local listener and bounded forward registry, supports an engine-selected local
port, and stops forwards on explicit removal, tab disposal where applicable, session
replacement, disconnect, or teardown. The workspace port-forward table retains
honest stopped/error states and does not treat a failed refresh as an empty list.

Behavior-only reference: local `/Volumes/Data/Kerjaa/freelens`, commit
`088d08513c8c149365dd802c588b9b0b90c6293e`; no upstream source was copied. Iceon
owners are `connection/resources.rs` and `connection/pod_sync.rs` for typed LIST/WATCH
projection, `connection/pods.rs` for protected details/YAML, `connection/pod_runtime.rs`
for logs/exec/attach/forwards, `connection/helm.rs` for bounded Helm subprocess
inventory, and `PodDock.tsx`, `ResourceTablePage.tsx`, `PodDetailsView.tsx`,
`TerminalView.tsx`, `HelmTablePage.tsx`, and `PortForwardTablePage.tsx` for presentation
and typed IPC adapters. Acceptance paths are `tests/resource-tables.spec.ts`,
`tests/pod-workspace.spec.ts`, and `tests/service-tables.spec.ts`, with Rust unit
coverage in `src-tauri/src/connection/resource_tests.rs`, `pod_sync_tests.rs`,
`pod_tests.rs`, `pod_runtime_tests.rs`, and `helm_tests.rs`. These paths record the
workflow contract; they do not establish real-cluster or native OS behavior.

| Source under `packages/core/src/renderer/components/` | Expected interaction / Rust owner | UI states / acceptance paths |
| --- | --- | --- |
| `workloads-pods/pods.tsx`, `workloads-pods/pod-details.tsx`, `dock/edit-resource/` | Pod activation, reduced details/events, protected YAML save and confirmed delete/force-delete/finalize; `connection/pods.rs` | Loading, stale retained details, missing/replaced UID, permission failure, conflict with edited YAML retained; `tests/pod-workspace.spec.ts`, `src-tauri/src/connection/pod_tests.rs` |
| `workloads-pods/store.ts` | Resource table loads normally; the separate **Live sync** control opt-in enables first-page LIST+WATCH with namespace scope, safe projection, relist and cleanup; `connection/pod_sync.rs` | Opening, syncing, retrying/stale, partial, denied/error, paused/stopped; `tests/pod-workspace.spec.ts`, `src-tauri/src/connection/pod_sync_tests.rs` |
| `dock/logs/controls.tsx`, `dock/logs/list.tsx`, `dock/terminal/view.tsx` | Container selection, bounded logs/search/download and remote exec/attach input/resize; `connection/pod_runtime.rs` | Opening, live, exited, error, explicitly dropped output, late-open cleanup; `tests/pod-workspace.spec.ts`, `src-tauri/src/connection/pod_runtime_tests.rs` |
| `dock/dock.tsx`, `dock/dock-tabs.tsx` | Reusable bottom dock, tab selection/close, minimize/maximize/resize; Rust session owns child runtime cleanup | Navigation/minimize preserves sessions, tab close disposes owners, disconnect/replacement tears down all; `tests/pod-workspace.spec.ts`, `src-tauri/src/connection/pod_runtime_tests.rs` |
| `helm-charts/helm-charts.tsx`, `helm-releases/releases.tsx`, `network-port-forwards/port-forwards.tsx` | Cached chart and scoped release inventory; forward inventory/start/stop; `connection/helm.rs`, `connection/pod_runtime.rs` | Loading, empty, local filtered-empty, stale/error, bounded/partial, stopped forwards; `tests/service-tables.spec.ts`, `src-tauri/src/connection/helm_tests.rs`, `src-tauri/src/connection/pod_runtime_tests.rs` |

### Reference workflow record

Behavior-only reference: local `/Users/user1/Kerjaa/freelens`,
`v1.10.3-533-gcd07118f`, commit `cd07118f843c4a69f937a319a37a7c12ef2c92d8`.
No upstream source code was copied. `iceon` is not the reference checkout.

| Workflow/source under `packages/core/src/` | Expected interaction / Rust owner | UI states / acceptance |
| --- | --- | --- |
| `renderer/components/catalog/catalog.tsx`, `columns/browse-all.injectable.tsx` | Combined entity inventory with kind/category, category filters; `CatalogEngine::list` | Loading, empty, loaded, filtered-empty, unavailable/error; `tests/catalog.spec.ts` aggregation and legacy redirect |
| `renderer/components/catalog/hotbar-toggle-menu-item.tsx` | Toggle shortcuts for entities, not categories; `set_pin` | Persisted pinned/unpinned, item-specific workspace or browser activation; Rust restart/dedup tests plus browser row-pin tests |
| `common/catalog-entities/kubernetes-cluster.ts` (`onRun`), `renderer/components/layout/sidebar.tsx`, `sidebar-cluster.tsx`, `common/sidebar-menu-items-starting-order.ts` | Cluster-ID route, scoped inset navigation, temporary Catalog collapse; Rust owns the session lease and bounded client/resource requests | Loading, disconnected, connecting, connected, permission/transport failure, stale resource data, missing cluster; `tests/catalog-activation.spec.ts` and `tests/cluster-connection.spec.ts` |

Default-path discovery and the six-column table/colors are Iceon requirements,
not a claim of complete Freelens behavior. Acceptance: Rust discovery/migration/
color fixtures and browser built-in discovery, warning, column/icon, and reload tests.
Catalog import does not authenticate or mutate Kubernetes objects; connection,
Pod watches, and Pod operations are separate scoped services described above.
Freelens extensions remain outside scope. Tests use synthetic fixtures and mocked IPC;
they do not prove production-cluster or native OS integration.

## Interaction and safety

### Current workspace UI reference

The scoped source/ownership/state/acceptance mapping is in
[`docs/workspace-ui-reference.md`](docs/workspace-ui-reference.md). It records
CodeJar 4.3.0/highlight.js YAML at the three existing editor locations, semantic
status colors, selection headings, Catalog action placement, and the layout-owned
resizable inspector. The inspector's CSS transitions honor reduced motion;
closing removes content and cancels reads immediately, leaving an inert collapsing
shell rather than an active hidden panel.

Workloads overview has seven donuts for bounded loaded inventories, not live
totals: at most five pages/500 workloads per kind, read sequentially. Lifecycle
labels use Running, Pending, Succeeded, Failed, Terminating, Scheduled and Suspended;
they do not claim readiness. Controllers use observed desired/ready counters,
with an omitted ready counter treated as zero only when desired is known.
Unlike Freelens's child-Pod association model, these are controller-counter
summaries. Unknown desired counts stay Unknown; failed Job attempts alone do not
establish terminal failure. Charts link to their inventory and keyboard-operable
color-key legends toggle segments without changing loaded totals.
Events load at most 20 pages/2,000 records. The sortable, ten-row paged table
defaults to newest creation first (Age), not last occurrence. Sorting applies
before paging across all loaded events; Last seen remains a separate timestamp.
Partial inventory remains visible and never claims cluster-wide latest events.
Warning messages use Astryx's semantic red text token, not arbitrary colors.
Events show message/source/count/times from Rust projection (text capped at 256
UTF-8 bytes with control/bidi filtering), not raw objects or universally
secret-free text. CRD inventory uses limit 10, same-token limit-1 retry on an
oversized response, a CRD-only 16 MiB single-object body bound, and a 64 KiB
projected-item budget. The ordinary 2 MiB response cap is unchanged. Custom
instances now have a separate metadata-only read path; schemas, custom-object
details and mutations are not exposed. See [custom resources](docs/custom-resources.md)
for discovery limits, verified CRD identity and separate cancellation slots.

Cluster Overview is distinct from Workloads overview. `ClusterOverview.tsx` reads
actual Nodes and Warning Events, bounded to one page of up to 100 items each,
through Rust. CPU/memory utilization comes from Metrics Server snapshots, using
valid allocatable values or capacity; incomplete pages/metrics do not become an
invented aggregate. A ten-row Warning Events table from the loaded snapshot uses
validated Event identities to resolve related-object/namespace details through
Rust. Loading, missing metrics, partial, stale and independent
failures remain visible. No Prometheus integration is configured: there are no
historical charts or fabricated time series. Final validation is pending; native
file-picker and actual-cluster smoke tests have not been performed.

Inspector fact groups and secondary tables use flat Astryx cards with headings
inside their boundary. Primary resource inventories remain compact unwrapped
tables. Routine scope explanations use named information popovers; errors,
stale results, redaction warnings and partial data stay visible. Memory usage
shares a theme-aware fill across Nodes and Cluster overview, distinct from its
track in both themes. This is used/allocatable, not a fabricated requested-resource
overlay. The workload heading and refresh indicator share a group so an inactive
indicator does not introduce a second section gap.

[Release dry run](docs/release-dry-run.md) documents the manual, CI-gated ARM64
ad-hoc packaging check without a version bump, tag, Release or updater artifacts.

- Data views distinguish not connected, loading, empty, filtered-empty, stale,
  permission denied, and failed requests. Preserve useful data during reconnects
  and identify stale values; never substitute fabricated metrics or success.
- Show cluster and namespace scope beside operational actions. Destructive
  actions require a specific target and confirmation; Rust owns execution.
- All controls work by keyboard, with visible focus. Dialogs close on Escape
  and return focus. Use named landmarks and status/error announcements.
- Verify WCAG AA contrast in every shipped mode. Color alone never expresses
  connection state, severity, or selection. Native/custom window controls remain
  usable at narrow widths and enlarged text sizes.

## Verification

Run `bun run build`, `bun run test:e2e`, Rust tests, formatting, and strict Clippy
when touching the frame or IPC. Browser tests mock native metadata/commands;
they do not establish OS behavior. Smoke-test the actual app on macOS, Windows,
and Linux for drag, resize, minimize, maximize/restore, close, fullscreen, and
safe-area placement before claiming platform support. Do not operate a real
cluster as a test without explicit approval.

## References

- [Freelens](https://github.com/freelensapp/freelens)
- [Freelens Catalog and navigation](https://freelensapp.github.io/docs/extensions/reference/08-catalog-navigation-theme/)
- [Tauri window customization](https://v2.tauri.app/learn/window-customization/)

Reference behavior must be checked against an identified upstream version when
implementing a workflow. These links describe reference material, not an
Iceon compatibility guarantee.

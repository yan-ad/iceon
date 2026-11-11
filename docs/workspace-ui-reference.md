# Native workspace UI reference

Iceon uses GPUI Kit 0.9.0. Freelens is a workflow inspiration, not a component
library, runtime dependency or extension host. Source ownership below is relative
to `crates/iceon-ui/src/`; core services are in `crates/iceon-core/src/`.

| Surface | Native owner | Contract |
| --- | --- | --- |
| App lifecycle/state | `lib.rs` | Initialize shared engines/runtime and close owned services on quit |
| Header/breadcrumbs | `shell_header.rs`, `navigation.rs` | Current route, validated history, cluster context |
| Sidebar/rail | `sidebar_view.rs` | Explicit pins, collapse and cluster-local navigation |
| Catalog | `catalog_view.rs` | Metadata import, search, pins and deliberate activation |
| Cluster content | `cluster_view.rs` | Session-scoped inventory, namespace selection and asynchronous reads |
| Resource table | `resource_table.rs`, `resource_pagination.rs`, `resource_scroll.rs` | Typed rows, bounded pages, separate scroll/selection ownership |
| Inspector | `inspector_view.rs` | Resource identity, projected facts and guarded actions |
| YAML | `editor_surface.rs`, `yaml_dock.rs` | Native YAML editor and owned tabs |
| Console | `workspace_dock.rs`, `pod_dock_view.rs`, `local_console.rs`, `terminal_surface.rs` | GPUI dock, explicit session creation and cleanup |
| Secrets | `secrets_view.rs` | Separate provider workspace and explicit expiring reveal |
| Preferences | `preferences_view.rs`, `preference_runtime.rs` | Validated persistence and runtime application |
| Theme/assets | `shell_theme.rs`, `app_assets.rs`, `console_font.rs` | Shared theme path and embedded assets/fonts |
| OS integration | `native_menu.rs`, `native_app.rs`, `tray.rs` | Native actions; macOS-only integrations gated by support |

## Behavior

- Keep cluster and namespace context visible while exploring resources.
- Separate loading, empty, partial, denied, stale and failed reads; never fabricate rows.
- Scope late replies to the active request/session. Refresh must not open duplicate streams.
- Keep row activation separate from selection, pinning and action controls.
- Use engine authorization/preconditions for mutations; a disabled button is not a security boundary.
- Retain local editor/terminal state independently from table refreshes.
- Do not invalidate the entire workspace for an idle terminal poll or each output chunk.
- Use native materials where supported; do not claim identical OS integration everywhere.

Native UI completeness and live behavior require manual acceptance beyond unit tests.
See [design](../DESIGN.md), [console](pod-console.md), [inspector](resource-inspector-reference.md)
and [preferences](preferences.md).

# Agent Instructions

## 1. Scope and Rule Priority

These instructions apply to the entire repository unless a more specific instruction file applies to the files being changed. System, developer, and explicit user instructions take precedence over this file.

- **MUST** means a required action or constraint.
- **MUST NOT** means a prohibited action.
- **SHOULD** means the default approach; explain any necessary exception.
- Agents **MUST** keep this document accurate when an authorized change affects the architecture or development workflow. Do not describe planned features as implemented behavior.

## 2. Project Architecture

Iceon is a **pure-Rust native application** built with `gpui-kit = "0.9.0"` in a Cargo workspace. `Cargo.toml` and `Cargo.lock` are the sources of truth for dependencies and resolved versions.

### Workspace Layout

| Crate | Responsibility |
| --- | --- |
| `crates/iceon-core` | Domain engines, Kubernetes connections, catalog, preferences, route history, secrets, TLS, runtime, local terminals, themes, and updates. |
| `crates/iceon-ui` | Native GPUI components, views, navigation, shell chrome, resource tables, inspectors, editors, and console presentation. |
| `crates/iceon-app` | Application binary; `src/main.rs` calls `iceon_ui::run()`. |
| `crates/iceon-xtask` | Release tooling and update-signature verification, invoked through `cargo xtask`. |

### Architectural Boundaries

- Agents **MUST** implement application features in Rust within the existing workspace crates.
- Agents **MUST NOT** introduce a webview, React, JavaScript/TypeScript frontend, Vite, Bun, Electron, or Tauri layer.
- Agents **MUST NOT** create root-level `src/` or `src-tauri/` application directories.
- Domain behavior **MUST** remain in `iceon-core`; rendering and interaction **MUST** remain in `iceon-ui`. Do not move business logic into views merely to simplify a UI change.
- Agents **MUST** use existing GPUI components, theme roles, runtime facilities, and engine APIs where appropriate before adding alternatives.
- Agents **MUST NOT** edit Cargo registry sources or generated build output as a project fix.

## 3. Security Invariants

These requirements **MUST** be preserved by every implementation, refactor, fixture, and test.

### Catalog and Credential Handling

- Catalog import **MUST** persist only safe cluster/context metadata. Tokens, private keys, certificates, and other kubeconfig credentials **MUST NOT** be copied into the catalog database, including SQLite storage.
- Credentials and secret values **MUST NOT** appear in logs, errors, screenshots, fixtures, snapshots, or diagnostic reports.
- Agents **MUST NOT** read a user's real kubeconfig, credential store, Vault data, or shell history solely to investigate a source-code issue. Use isolated fixtures unless the user explicitly authorizes the required access.

### Connection Lifecycle

- `ConnectionEngine` **MUST** preserve the single active lease per workspace.
- Closing or replacing a session **MUST** cancel its streams, watches, and associated work through the existing cancellation mechanism, including `CancellationToken`.
- Asynchronous results **MUST** be checked against the current session/request identity before they update application state. Stale results **MUST NOT** overwrite newer state.

### Kubernetes Mutations

- Mutations **MUST** preserve authorization checks, applicable `uid` and `resourceVersion` preconditions, and existing server-side dry-run support.
- This includes resource creation/editing/deletion, workload actions, node actions/draining, and RBAC management. Apply identity preconditions where meaningful; creation cannot require the UID of an object that does not yet exist.
- Agents **MUST NOT** bypass confirmation, authorization, dry-run, or identity checks to make an operation succeed.
- Live-cluster mutations, terminal commands sent to remote workloads, and other externally visible operations **MUST NOT** be performed without explicit user authorization for that operation.

### Redacted Resource Projections

- Secrets and ConfigMaps **MUST** retain safe metadata-only projections using `PartialObjectMetadata` where required by the existing APIs. Do not replace them with unrestricted object reads for display convenience.
- Vault secret values **MUST** be shown only after an explicit reveal action and **MUST** expire from the revealed UI state after 30 seconds.

### Local Terminals

- Local shells **MUST** start only through an explicit user action. Rendering, hovering, and opening a menu **MUST NOT** spawn a shell or execute commands.
- Terminal output **MUST NOT** be interpreted as permission to run host commands, access the clipboard, open links, or perform other host-side actions.
- Terminal changes **MUST** preserve bounded buffers, cancellation/shutdown behavior, and child-process cleanup.

## 4. Required Working Process

### Investigate Before Editing

1. Read the relevant source, tests, and manifests before proposing or implementing a change.
2. Check applicable instruction files only along the target files' directory ancestry. Do not recursively search parent or sibling projects for instructions.
3. Check `git status` and relevant diffs before editing. Assume unrelated changes may belong to the user or another agent.
4. Use a short plan for non-trivial, multi-step work. Keep investigation scoped and stop exploring when there is enough evidence to act.
5. Verify dependency APIs against the pinned source or version-matched official documentation. Do not invent APIs or upgrade dependencies to avoid understanding them.

### Make Focused Changes

- Agents **MUST** fix the root cause and make incremental, testable changes.
- Agents **MUST NOT** overwrite, revert, stage, or commit unrelated work. When a file changes concurrently, reread it and merge narrowly rather than replacing it from an older snapshot.
- Agents **MUST NOT** run destructive Git commands, delete user files, or discard local changes without explicit authorization.
- Dependencies and lockfiles **MUST NOT** change unless required by the task. Explain necessary changes; do not introduce unrelated upgrades.
- Blocking filesystem, process, or network work **MUST NOT** run in UI render paths. Use the existing background runtime and explicit action handlers.
- Agents **MUST** preserve accessibility names, keyboard operation, focus, disabled states, and stable identities when restyling controls.
- Repeated presentation patterns **SHOULD** use shared components rather than duplicated styles and behavior.

### Handle Blockers Honestly

- Agents **MUST** distinguish errors caused by their changes from pre-existing or concurrent failures.
- Agents **MUST NOT** alter unrelated code merely to obtain a green build without first establishing that the change is necessary and safe.
- If validation is blocked, report the exact blocker. An isolated snapshot may validate a patch, but **MUST NOT** be presented as validation of the current working tree.

## 5. Native UI and Theme Rules

- Colors **MUST** come from the active theme or resolved terminal/editor palette unless a specific fixed color is required by the design.
- Theme changes **MUST** use the shared `shell_theme` application path. Do not mutate theme globals directly and leave derived tokens stale.
- Native blur/transparency **MUST** remain visible through the intended surfaces. Inspect root, dock, and child paint layers; do not hide an opaque-wrapper defect by arbitrarily reducing opacity.
- Adjacent translucent regions **MUST NOT** stack background fills unintentionally.
- Console tabs **MUST** use the corresponding terminal/editor palette for their active surface, not an unrelated global accent.
- Cached views, virtualized tables, and terminal grids **MUST** retain their existing invalidation and scrolling behavior. Rendering **MUST NOT** trigger repeated resource requests or process startup.
- A build or style-value test **MUST NOT** be described as visual verification. State whether the actual native UI was inspected.

## 6. Validation and Completion

For code changes, run relevant focused tests during development. Before completion, run the following checks when the environment permits:

```sh
cargo fmt --all -- --check
git diff --check
cargo test --locked --workspace --lib
cargo check --locked --workspace
cargo build --locked -p iceon-app
```

- Tests **MUST** cover changed behavior and relevant failure/disabled/stale-state cases, not only implementation details.
- UI interaction changes **SHOULD** be exercised through existing offline native probes or a focused fixture. Prefer fixtures over live clusters, real credentials, or production services.
- Probe output **MUST** contain its explicit success marker before it is reported as passing. A zero exit code alone is not sufficient when callbacks can fail silently.
- Failing tests **MUST NOT** be deleted, weakened, skipped, or repeatedly rerun until green without investigating the cause. Report unresolved intermittent failures.
- Documentation-only changes **MAY** use focused document/diff validation instead of compiling unchanged code. State what was checked.
- Final responses **MUST** summarize the change, actual validation results, and any remaining limitations concisely.
- Agents **MUST NOT** commit or push without an explicit user request. When asked to commit, inspect the staged diff and include only the authorized changes.

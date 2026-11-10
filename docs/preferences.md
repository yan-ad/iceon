# Preferences behavior

Preferences has five flat sections: **App, Proxy, Kubernetes, Editor, Terminal**.
Changes are persisted through the revision-checked preferences store. Validation or
OS-integration failures keep the previous saved setting and show an error.

## App

- Theme follows the native window appearance in System mode, including subsequent
  OS appearance changes. Explicit Light/Dark overrides it.
- The searchable timezone selector uses Jiff's IANA database. Inspector and log
  timestamp presentation uses the selected timezone; source/exported log text is
  not rewritten. Relative ages remain relative.
- Persistent Search keeps resource text filters across resource-kind navigation;
  disabled clears the text filter when switching kinds.
- Restore Last Page uses validated persisted route history at startup.
- Open at Login writes/removes an Iceon-owned macOS LaunchAgent. It does not launch
  another instance immediately. Unsupported platforms disable the control.
- Auto-hide Hotbar collapses the cluster icon rail to an 8px hover target.
- Show Tray Icon creates/removes a real macOS status item. Clicking it activates
  the existing main window. Unsupported platforms disable the control.
- Reset Sidebar Order restores the current cluster rail to catalog order.

## Proxy

HTTP(S) proxy and Allow Untrusted Certificate Authorities apply to the dedicated
kubectl download client and Helm chart/repository traffic. The certificate switch
skips certificate validation for those requests and should normally remain off.
It does **not** disable Kubernetes API certificate verification. No process-wide
proxy environment is mutated. There is currently no local host-terminal workflow.

## Kubernetes

- Automatic kubectl downloads are opt-in. **Prepare kubectl for connected cluster**
  first resolves configured local executables, otherwise queries the connected
  cluster's version and downloads the matching upstream release when enabled.
- Default/custom mirror, binary directory, and explicit executable path are used
  by the resolver. Downloads are bounded, SHA-256 checked, atomically installed,
  and cancelled when the cluster lease changes. Success copies the executable
  path to the clipboard. Preparation never executes downloaded binaries or edits
  the host PATH. Pod exec remains a direct Kubernetes API operation.
- Helm inventory/detail/install operations honor the configured Helm executable
  and application proxy. Server-side apply is a real Helm 4 option; requesting it
  with Helm 3 returns an error instead of silently ignoring it. Server dry runs
  remain server-side independently of the apply preference. The core upgrade API
  also supports the same apply policy and release preconditions.
- Repository Refresh/Add/Remove invoke the real Helm CLI without requiring a
  cluster session. Only successful command/read-back results are projected into
  the saved repository snapshot. There is no fabricated repository catalog.
- Default discovery and configured kubeconfig files/shallow directories are
  reconciled immediately and every five seconds. Only cluster/context metadata
  enters the catalog. Invalid/unavailable sources retain last-valid metadata;
  confirmed removals remove sync-owned entries, not manually managed imports.

## Editor

Font family/size and tab size apply to the native YAML editors without replacing
editor entities or their undo history. Line numbers implement On, Off, Relative,
and Interval modes. The left/right minimap is derived from real buffer content,
tracks the viewport, and supports click/drag/wheel navigation.

## Terminal

Saved shell is used for **new Pod sessions** and must exist inside the selected
container. Empty uses `/bin/sh`; it is not a host shell preference. Font family,
size, and Match/Dark/Light theme affect the terminal/log viewers. The font selector
lists installed fonts, including registered bundled fonts. Text selection and
copy-on-select are implemented; right-click paste and keyboard copy/paste remain
explicit terminal-scoped actions.

## Intentionally omitted

Iceon has no extension installation subsystem, so an npm/extension registry
preference is not exposed. Download mirroring, Helm repositories, and kubeconfig
sync are working workflows, not substitutes for an extension installer.

## Verification

Core tests cover preference migration/validation/rollback, metadata-only sync,
lease cancellation, binary resolution/download/hash validation, and real Helm
command arguments using fixture executables/servers. UI tests cover presentation
helpers, editor modes/minimap mapping, terminal parsing, tray lifecycle, and exact
navigation section order. Native tray clicks, login-at-next-session, and visual
layout still require an interactive macOS smoke test; automated unit/build checks
do not establish those OS interactions.

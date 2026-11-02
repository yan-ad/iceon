# Changelog

## 0.1.0 (pending CI)

### Secrets management

- Add a standalone Secrets workspace beside Clusters, with a + action opening
  Add secret connection. Catalog remains cluster-only; Secrets is not a Preferences
  section and provider connections are not cluster pins.
- Add provider-neutral Rust transport and credential-store interfaces, with Vault
  userpass login and read-only KV v2 browsing as the first provider. Saving a profile
  stores metadata only and does not authenticate. Secret values require explicit
  Reveal and are hidden on blur, navigation, disposal, or after 30 seconds.
- Remember only issued Vault tokens using macOS Keychain, Windows Credential
  Manager, or Linux Secret Service. Passwords are not persisted; no plaintext
  credential fallback is provided. Resume validates saved tokens before reading.
- Bound requests and responses, require verified HTTPS, disable redirects, isolate
  sessions by native window, and cancel stale authentication and reads. Add explicit
  expiry, disconnect, forgetting, and profile removal cleanup. No remote secret
  writes, automatic renewal, exports, or Kubernetes credential injection.
- Document setup, policy requirements, supported limits and manual native checks in
  `docs/secrets-management.md`.

### Workspace fixes

- Use Astryx empty states for resource and port-forward tables, with recovery
  actions, scope-aware guidance and responsive horizontal-scroll behavior.
- Use asynchronous native kubeconfig file selection instead of a blocking wrapper.

### Verification and distribution

- Local feature verification passed: 533 Rust tests, 70 navigation/UI fixture tests,
  a final 26-case Secrets/route regression run, production build, formatting and
  strict Clippy. Fixture tests do not establish live Vault or native credential-store
  behavior; those manual checks remain outstanding on each supported OS.
- The immutable `0.1.0` tag starts the existing gated macOS/Windows packaging and
  signed-updater workflow. Publication still requires same-commit CI, package and
  uploaded-artifact verification; a pushed tag is not proof of a published release.

## 0.0.8 (pending CI)

### Kubeconfig import

- Keep Add config compact, with Choose file inline beside the path. A single
  native dropped-file path selects the file only; it never imports automatically.
  Import remains an explicit, metadata-only action, not authentication or connection.
- Add optional grouped platform guidance for managed providers (GKE, DOKS, LKE,
  VKE, OKE, IBM Cloud Kubernetes, Alibaba ACK, Tencent TKE, Scaleway Kapsule,
  OVHcloud and Civo), self-managed platforms (Hetzner + K3s, OpenShift, Rancher,
  RKE2 and K3s), and local clusters (MicroK8s, kind and minikube). Guidance helps
  prepare an existing kubeconfig; it does not enumerate cloud accounts or add
  authentication, login, SSH or tunnels.
- Add synthetic compatibility fixtures for provider-neutral import and connection
  preparation, source identity, credential redaction and insecure-transport
  rejection. These are not live-provider authentication or certification tests.

### Other changes since 0.0.6

- Add conservative Node drain with read-only review, explicit Node-name
  confirmation, single-use expiring plans, guarded cordon and policy/v1 evictions.
  Partial inventories and unsupported/local-data Pods block execution; no force,
  PDB bypass or direct-delete fallback is provided. Completion requires observed
  Pod removal; cancellation cannot undo submitted writes.
- Expand bounded inspector facts, related collections, Event diagnostics, Helm
  details and metadata-only custom-resource snapshots. Add reduced resource
  editing with dry-run/confirmation, guarded workload actions and Service-aware
  forwarding. These are scoped workflows, not full-object editing or feature parity.
- Improve namespace continuity/caching, Catalog-aware search, cluster/section
  pins and native pin ordering, sidebar resizing and inspector/dock actions.
  Remove Web Links; Catalog navigation now contains Clusters only.
- Download verified updates automatically but defer installation/restart until
  **Restart to Update**. Reduce redundant updater-context renders; native CPU
  savings have not been measured. Add an authenticated development-only browser
  engine bridge, not a production Node backend.

### Verification and distribution

- Group inspector facts and supporting overview/related tables in Astryx cards.
  Keep primary inventory tables compact. Repair invisible memory usage in dark
  mode and move routine overview explanations into information popovers.
- Use lifecycle terminology for workload charts, compact interactive color-key
  legends and tighter heading spacing. Add paged/sortable loaded events, default
  newest-creation ordering and semantic warning message colors. Controller
  summaries use replica counters, not Freelens's child-Pod health model.
- Repair strict IPC fixtures, responsive toolbar selectors and permission-expiry
  timing. Export same-commit Rust collection contracts alongside detail contracts
  for browser CI instead of compiling Rust on frontend-only runners.

Local validation for the 0.0.8 retry passed: 1,176 Chromium fixture tests
(four production-only skips), six synthetic dev-engine tests, 473 Rust backend
tests, 16 release-tool tests, 18 publisher tests and five Bun tool tests, plus
formatting, production build and strict Clippy. GitHub CI/package validation
remains the publication gate.
Native file drops and Windows runtime behavior still need smoke
validation; no live-provider, real-credential or live-cluster success is claimed.
Full CI and both platform packages, verified signed updater packaging and
uploaded-byte checks gate publication. macOS remains ad-hoc signed and not
notarized; Windows has no Authenticode certificate. Native installation/update
smoke checks remain outstanding. See [installation](docs/releases.md).

## 0.0.6

### Fixed

- Fix a fish Google Cloud SDK discovery case missed by 0.0.5: installer-style
  `if [ -f '…/path.fish.inc' ]; . '…/path.fish.inc'; end` hints are now
  recognized, including quoted `~/` paths. The tested path and sourced path must
  match. No condition, shell startup command, SDK script, or authentication
  plugin is executed during discovery.
- Recognize bare `.` SDK source hints as well as `source`. Quoted `~/` is treated
  only as a home-relative SDK installation hint; this does not claim fish expands
  the original declaration. Additional commands, substitutions, mismatched paths,
  and comments that hide the source or closing statement are rejected.

### Verification and distribution

Regression fixtures reproduce the exact guarded source declaration that 0.0.5
missed, including helper and sibling SDK command execution with synthetic data.
A separate read-only check with a minimal Finder PATH resolved the locally
installed helper from the actual fish hint without running it. Real kubeconfigs,
credentials, and live clusters were not accessed. Full CI and both verified
platform packages gate publication. Existing signed-update installations can
receive this patch automatically; native install/update smoke tests are not claimed.

## 0.0.5

### Fixed

- Fix Google Cloud SDK authentication-helper discovery for macOS Finder/Dock
  launches when the SDK path is configured in fish. Iceon now recognizes literal
  SDK paths in `config.fish`, bounded `conf.d` snippets, and saved
  `fish_user_paths`, including custom locations such as
  `$HOME/Kerjaa/google-cloud-sdk/bin`. A persisted fish declaration no longer
  requires launching Iceon from a terminal with a manually exported PATH.
- Preserve OS search order for an explicit kubeconfig PATH, including relative
  entries. Automatic discovery does not replace an explicit helper command or
  modify kubeconfig, shell configuration, or the application's global environment.
- Keep discovery bounded and non-executing: shell startup commands are never run,
  unsupported expressions are ignored, and existing helper deadlines and
  cancellation remain enforced. A PATH change made only in an open terminal
  cannot be recovered by a Finder-launched app; custom paths must be persisted.

### Verification and distribution

Local checks passed 357 Rust tests and 848 browser fixture tests, plus frontend
build, formatting, strict Clippy, and an unsigned macOS release bundle build.
Fixtures cover fish discovery, saved paths, sibling SDK commands, explicit PATH
precedence, malformed syntax, read limits and cancellation. Real credentials,
live clusters, and native installation/update smoke tests were not used.

Full CI, both platform packages, signed updater verification, and uploaded-byte
checks gate publication. Working 0.0.4 installations can receive this patch
through automatic updates. If 0.0.3 crashes at startup, or when upgrading from
0.0.1/0.0.2, install the new package manually. macOS remains ad-hoc signed and not
notarized; Windows has no Authenticode certificate. See
[installation](docs/releases.md) and [automatic updates](docs/automatic-updates.md).

## 0.0.4

### Fixed

- Fix a startup SIGABRT in supported release builds, reported on macOS 27.0.1.
  The updater could construct reqwest's HTTPS client before a rustls crypto
  provider was initialized. Release-mode panic handling then aborted the app on
  a Tokio worker. Iceon now initializes the ring provider before Tauri startup
  and also ensures initialization in the updater transport constructor.
- Add fresh-process negative and positive regression controls using the actual
  HTTP client constructor, plus concurrent initialization and startup-order tests.
  These checks make no network requests and do not launch the native app.
- Keep certificate verification, pinned updater signatures, signed-version
  enforcement, icons, colors, and existing Kubernetes behavior unchanged.

### Installation and verification

If 0.0.3 crashes immediately, install 0.0.4 manually: the affected build aborts
before its automatic updater can retrieve a patch. Versions 0.0.1/0.0.2 also
require manual bootstrap because they shipped without the updater verification
key. Supported working builds continue to use signed GitHub update assets.

Apple Silicon macOS DMG and Windows x86 setup.exe packages are gated by full CI,
platform builds, signature/version verification, and uploaded-byte checks. The
app is not Apple notarized or Windows Authenticode signed. Only for a trusted
macOS download installed in `/Applications`, use:

```sh
xattr -dr com.apple.quarantine /Applications/Iceon.app
```

Do not disable Gatekeeper or SmartScreen. Native installation and live-cluster
smoke tests are not claimed. See [installation](docs/releases.md) and
[automatic updates](docs/automatic-updates.md).

## 0.0.3

### Included changes

- Automatic application updates from the public GitHub stable feed, with a pinned
  embedded minisign public key and authenticated package-version checks. Rust
  bounds downloads and uses the official Tauri bundle/installer update path;
  unsigned packages, invalid signatures and mismatched versions are rejected.
- Safe automatic installation and restart after every admitted window freezes
  interaction and acknowledges its current preparation ID. New-window admission
  closes during preparation. Dirty editors, dialogs, active mutations, terminals
  and port forwards defer installation; backend session work drains before install.
  Preparing and Restarting are reported without saving or changing user drafts.
- A Windows 32-bit x86 NSIS setup route alongside the Apple Silicon macOS DMG.
  Windows updates use quiet, current-user installation. WebView2 is downloaded
  if missing, which requires a network connection.
- macOS credential-helper discovery first resolves a canonical executable from
  the helper's child PATH. After a not-found result, bounded fallback reads
  `/etc/paths`, at most 32 `/etc/paths.d` entries and five startup files for
  literal Google Cloud SDK source hints only. Files are never executed; reads
  are limited to 8 KiB per file and 32 KiB total. No helper download or automatic
  cloud login is performed; genuinely missing helpers remain errors.

### Verification and distribution

Local checks passed 348 backend tests, 16 release-tool tests, and 252 updater,
authentication and active-work UI regressions, plus build and strict Clippy. Full
CI and both platform packages gate publication. Native installation/update smoke
checks remain outstanding; this entry makes no Windows-native or real-credential
provider success claim. Existing RBAC behavior, SIGHUP credits,
branding, icons and record colors are unchanged by this release.

The release publishes exactly seven assets: `Iceon_0.0.3_aarch64.dmg`,
`Iceon_0.0.3_aarch64.app.tar.gz`, its `.sig`, `Iceon_0.0.3_x86-setup.exe`,
its `.sig`, `latest.json` and `SHASUMS256.txt`. SHA-256 checksums and GitHub
asset digests detect transfer corruption; updater trust comes from the package
signature and its authenticated version, not the checksum file.

### First installation and OS warnings

Install 0.0.3 manually once, including when upgrading from 0.0.1 or 0.0.2:
those versions shipped without an updater trust key and cannot acquire automatic
updates retroactively. Subsequent verified versions install automatically when
safe, without a separate manual installation step.

macOS is ad-hoc signed, not Developer ID signed or notarized. Only for a trusted
download installed in `/Applications`, use
`xattr -dr com.apple.quarantine /Applications/Iceon.app` if macOS blocks launch.
Do not disable Gatekeeper globally. Windows has no Authenticode certificate;
SmartScreen may report an unknown publisher. No SmartScreen bypass is provided.
See [installation](docs/releases.md) and [automatic updates](docs/automatic-updates.md).

## 0.0.2

### Included changes

- Permission-aware resource navigation and scoped action menus distinguish
  allowed, denied, checking, stale, and unknown access. Read-only means a known
  allowed read with known denied relevant writes; unknown access is labeled
  Access unverified, not Read-only. Kubernetes remains the authorization boundary.
- Access Control > Users is an actual managed ServiceAccount workflow, not only
  an observed-subject directory: select a namespace, create or edit assignments
  from existing template ClusterRoles, review exact rules and a server dry-run
  plan, then confirm apply or deletion. New accounts disable token automount.
  Plans expire after 120 seconds, are consumed once, and bind session,
  authorization epoch and UID/resourceVersion baselines. Partial execution stops
  without automatic rollback. No namespace/role seeding, User CRD, token Secret,
  credential export or human identity-provider account CRUD is included.
- Credit SIGHUP's BSD-3-Clause Permission Manager as a pinned workflow reference;
  no upstream source was ported and Iceon's GPL v3 license is unchanged.

### Fixes since 0.0.1

- macOS credential-helper path resolution, including the GKE helper, accounts
  for GUI launch environments using bounded known installation locations without
  a login shell or directory scan. This work landed in
  `69ec0c2938fa3be1a1af9b98e28117ba1dd4c080`; it does not install a helper or
  guarantee that any cloud login is available.

### Verification and distribution

Local validation passed 328 backend Rust tests, 9 release-tool tests, and the
managed-user, permission, lifecycle and mutation fixture regressions. Build,
formatting and strict Clippy checks passed. Full cross-platform CI and package
verification gate publication. Native installation, real-cluster RBAC and
credential-helper smoke testing were not performed. The published 0.0.1
installer and its warnings remain unchanged.
See [permission management](docs/permission-management.md) for scope and
acceptance criteria, including limits on permission coverage.

### Download and installation warning

The installer is `Iceon_0.0.2_aarch64.dmg`, containing `Iceon.app` for Apple
Silicon Macs only. Publication requires full CI and packaging checks.
The app is **ad-hoc signed, not Developer ID signed or notarized**; macOS may
block it. No Intel, Universal, Linux or Windows installer is included. Automatic
updates are disabled; no updater archive, updater signature or `latest.json`
is published by the unsigned route.

Download the DMG and `SHASUMS256.txt` from this exact release.
Verify the download in its directory:

```sh
shasum -a 256 -c SHASUMS256.txt
```

Checksums detect corruption, not publisher identity. Open the DMG and drag
`Iceon.app` into `/Applications`. **Only if you trust this repository and this
downloaded release**, remove quarantine from the installed app and open it:

```sh
xattr -dr com.apple.quarantine /Applications/Iceon.app
open /Applications/Iceon.app
```

Do not disable Gatekeeper globally or remove quarantine from untrusted downloads.
Connect only to clusters and credentials you intend the application to use.
The published `0.0.1` tag and installer are not overwritten by this release.

## 0.0.1

Iceon is a Kubernetes desktop workspace. This release uses the Iceon name for
the application, repository links, executable, bundle identifier, and native menus.
It uses `ad.yan.iceon`; data from older application identifiers is not migrated
automatically.

### Included in this release

- Catalog navigation, search, metadata-only kubeconfig import, default-file
  discovery, saved web links, record colors and individual item pins.
- Rust-owned connection and resource workflows with scoped session identities,
  bounded resource inventory and safe detail projections, plus supported Pod,
  Helm and custom-resource workflows. Importing kubeconfig metadata does not
  authenticate or connect to a cluster. Opening a workspace starts its configured
  connection flow.
- Resource inspector loading, empty, partial, stale and error states. Secret and
  ConfigMap bodies and arbitrary custom-object payloads are not exposed as a
  generic raw-data detail view.
- An Apple Silicon macOS application and a headless DMG packaging route gated by
  the full reusable Rust and browser-fixture CI suite.

### Download and installation warning

The only installer is `Iceon_0.0.1_aarch64.dmg`, for Apple Silicon Macs. It is
**ad-hoc signed, not Developer ID signed or notarized**. macOS Gatekeeper may
block it. There is no Intel, Universal, Linux or Windows installer in this
release. Automatic updates are disabled. No updater archive, updater signature
or `latest.json` is published.

1. Download the DMG and `SHASUMS256.txt` from this exact release. In the download
   directory, verify them with:

   ```sh
   shasum -a 256 -c SHASUMS256.txt
   ```

   Checksums detect corruption, not publisher identity.
2. Open the DMG and drag `Iceon.app` into `/Applications`.
3. **Only if you trust this repository and this downloaded release**, remove
   quarantine from this installed application and open it:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Iceon.app
   open /Applications/Iceon.app
   ```

Do not disable Gatekeeper globally or remove quarantine from untrusted downloads.
Connect only to clusters and credentials you intend the application to use.

### Limits and verification

This is an early stable version number, not a claim of full Freelens parity or
extension compatibility. Resource details remain bounded safe projections,
not complete upstream resource editors. Generic detail metrics, live related
resource lists, full Helm release editing and arbitrary custom-object editing
are not established as equivalent workflows.

CI uses fixtures and mocked IPC, not production credentials or real clusters.
Packaging checks inspect signatures, ARM64 architecture, application metadata,
DMG contents and uploaded bytes. They do not prove native window/file-picker
behavior, Gatekeeper acceptance, actual installation, or live cluster behavior.
A real Apple Silicon native installation and interactive smoke test remains
outstanding; no such native verification is claimed by these notes.

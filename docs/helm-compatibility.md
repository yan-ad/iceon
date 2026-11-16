# Helm CLI compatibility

Iceon invokes a user-selected Helm executable, not a bundled SDK. This audit is
bounded to the implemented command surface and upstream **v3.17.0 / v4.0.0**
source contracts. It is not a guarantee for every historical, future, prerelease,
plugin-provided or vendor-modified Helm binary.

## Supported surface

Install/upgrade require stable Helm **3.17+ within major 3**, or **major 4**.
The existing supervised `version --template '{{.Version}}'` preflight now parses
complete semver and rejects malformed versions, prereleases, Helm 2 and unknown
majors before mutation. Version output may have a leading `v` and build metadata.
Read/repository commands use the common CLI contract below without a version
preflight; Helm 2 is not a supported target even if a particular read happens to
work. Existing deadlines, bounded output, cancellation and child cleanup apply.

| Implemented operation | Arguments / format | Helm 3 / Helm 4 mapping |
| --- | --- | --- |
| Chart inventory | `search repo -o json` | Shared. Exact exit-1 `Error: no repositories configured` becomes `[]`. No repositories means an empty chart inventory; add a repository in Helm preferences to populate it. |
| Chart metadata | `show chart REPO/CHART --version VERSION` | Shared YAML metadata, identity-checked and allowlisted; never `show all` or values. |
| Releases | `list` with `--deployed --failed --pending --uninstalled --uninstalling --superseded`, namespace or `--all-namespaces`, `--output json` | Shared explicit statuses preserve the Helm 3 all-status mask; Helm 4 removed `--all` and changed defaults. Existing list-limit truncation remains explicit. |
| Release history | `history NAME --namespace NS --max 100 --output json` | Shared JSON. Upgrade identity preflight also uses `--max 1` and checks revision. |
| Install | `install NAME REPO/CHART --namespace NS --values FILE --output json --timeout 60s`, optional `--version=VERSION` | Shared. Values are private temporary files; process output is discarded. |
| Upgrade | Same base options with `upgrade` | Shared; preserves displayed revision, Secret UID/resourceVersion and authorization preflight. No install fallback. CLI preflight is not an atomic compare-and-swap. |
| Dry-run install/upgrade | `--dry-run=server --hide-secret` | Shared string mode, not a boolean dry-run. Never retried as a mutation. |
| Wait install/upgrade | Helm 3 `--wait`; Helm 4 `--wait=legacy` | Explicitly preserves Helm 3 readiness strategy; bare Helm 4 `--wait` selects its new watcher. |
| Server-side apply / force conflicts | Helm 4 `--server-side=true/false`, optional `--force-conflicts` | Helm 4 install accepts bool; upgrade accepts `true/false/auto`. Force conflicts implies server-side. Requested SSA/conflict policy on Helm 3 fails closed, never silently downgraded. |
| Repository add | `repo add NAME URL`, optional `--insecure-skip-tls-verify` | Shared, explicit user action; HTTP(S), credential-free URLs only. |
| Repository list | `repo list --output json` | Both return `[]` for no repositories in JSON mode. Exact legacy exit-1 `Error: no repositories to show` is also accepted only on this read. |
| Repository remove | `repo remove NAME` | Shared; missing repositories remain a failure, not empty-success. |
| Rollback / uninstall | **Not implemented as Helm CLI actions** | No compatibility or execution claim. Existing Kubernetes resource actions are separate. |
| Repository update | **Not implemented** | No implicit update/network retry is added. |

The untrusted-CA preference adds the shared chart-download TLS flag to chart
metadata/install/upgrade and repository add; it never disables Kubernetes TLS.
Proxy preferences continue to bypass the exact Kubernetes host. No new flags,
authorization bypasses, ownership changes or automatic mutation retries are
introduced. Neither `--atomic` (Helm 3) nor `--rollback-on-failure` (Helm 4) is
emitted: automatic rollback/uninstall would change the approved operation.

## Error safety

Only the exact known empty-repository diagnostic on its corresponding read,
with exit code 1, is normalized. Trailing CR/LF is allowed; additional messages,
unknown flags, other exit codes and mutation errors are not swallowed. All other
stderr stays bounded and private, translated into fixed existing error categories
(TLS/auth/permission/conflict/timeout/connectivity or `HelmCommandFailed`). No
arbitrary CLI diagnostics, values, manifests or credentials are exposed to the UI.
An empty-repository screenshot is therefore not evidence of an incompatible CLI;
other `HelmCommandFailed` causes remain possible and are not guessed away.

## Verification and limitations

Offline shell executable fixtures exercise the exact chart/search/history,
release status and namespace argument contracts; repository preferences and
add/list/remove; install/upgrade under both major-version profiles; dry-run and
SSA/wait mapping; malformed/unsupported versions; exact empty-state errors versus
mutation/additional-diagnostic failures; output limits, cancellation and cleanup.
Run `cargo test --locked -p iceon-core --lib helm`.

These are synthetic contract fixtures, **not executions of real Helm binaries**.
No real kubeconfig, local Helm repository configuration, cluster, credential store
or remote chart endpoint was accessed. Source review does not establish end-to-end
chart/Kubernetes compatibility. Native UI visual verification is separate.

## Upstream evidence

Pinned official sources reviewed (same filenames under both tag roots):

- [Helm 3.17 command source](https://github.com/helm/helm/tree/v3.17.0/cmd/helm)
- [Helm 4.0 command source](https://github.com/helm/helm/tree/v4.0.0/pkg/cmd)
- `list.go`, `search_repo.go`, `show.go`, `history.go`, `install.go`, `upgrade.go`,
  `flags.go`, `repo_add.go`, `repo_list.go`, `repo_remove.go`.
- [Helm 4 wait strategy and chart flags](https://github.com/helm/helm/blob/v4.0.0/pkg/cmd/flags.go)
- [Helm 4 install flags](https://github.com/helm/helm/blob/v4.0.0/pkg/cmd/install.go)
- [Helm 4 upgrade flags](https://github.com/helm/helm/blob/v4.0.0/pkg/cmd/upgrade.go)
- [Helm 3 dry-run and hide-secret flags](https://github.com/helm/helm/blob/v3.17.0/cmd/helm/install.go)
- [Helm 3 empty search diagnostic](https://github.com/helm/helm/blob/v3.17.0/cmd/helm/search_repo.go)
- [Helm 4 empty search diagnostic](https://github.com/helm/helm/blob/v4.0.0/pkg/cmd/search_repo.go)

Rollback/uninstall sources were also reviewed to confirm they are distinct CLI
surfaces, but no new action was implemented or run.

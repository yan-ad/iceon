<div align="center">
<img src="./assets/icons/128x128.png" alt="Iceon" width="60" height="60">

# Iceon
### A native Kubernetes workspace, built entirely in Rust.
Explore clusters. Debug workloads. Stay in context.

[![Build](https://img.shields.io/github/actions/workflow/status/yan-ad/iceon/release.yml?style=flat-square&label=build)](https://github.com/yan-ad/iceon/actions/workflows/release.yml)
[![License](https://img.shields.io/github/license/yan-ad/iceon?style=flat-square&label=license&cacheSeconds=3600)](./LICENSE)
![Installs](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fad.yan.iceon%2Fcount&query=%24.count&label=Installs&color=blue&style=flat-square)
![Rust](https://img.shields.io/badge/Rust-2024-f74c00?style=flat-square&logo=rust)
![GPUI](https://img.shields.io/badge/GPUI-native_UI-1a43bf?style=flat-square)
![Status](https://img.shields.io/badge/status-early_development-e9a23b?style=flat-square)
[![Release](https://img.shields.io/github/v/release/yan-ad/iceon?style=flat-square)](https://github.com/yan-ad/iceon/releases)

</div>

## Already built

Iceon is past the native-foundation stage: the Rust/GPUI app has working cluster
and resource workflows, not just a shell. It is still early software; implementation
is not a claim of production readiness or complete provider/platform validation.

- **Clusters:** kubeconfig discovery/import, pinned destinations, namespace navigation,
  search, connection lifecycle and health checks.
- **Resources:** tables, details/events, YAML inspection/editing, server dry-run and
  confirmed mutations; custom-resource discovery and metadata browsing.
- **Debugging:** Pod logs, exec/attach, port forwards, local PTY terminals and a resizable console dock.
- **Operations:** Helm inventory/install and guarded RBAC management; some core APIs
  still need fuller native UI exposure and live-cluster acceptance coverage.
- **Desktop & secrets:** themes/preferences, native macOS menus/tray, and read-only
  Vault KV v2 browsing with explicit, time-limited reveal and OS credential storage.

## How fast is it?

An overall resource-usage snapshot of the installed apps, measured with Python:

| Metric | Freelens | Iceon | # |
| --- | --- | --- | --- |
| CPU Usage (mean, % of one core) | 1.89% | 0.16% | 12.1× lower CPU usage |
| Memory Usage (median physical footprint) | 633 MiB | 209 MiB | 3.0× lower memory usage |

Measured on an Apple M4 with 16 GiB RAM, macOS 27.0.1, using Freelens 2.0.0
and Iceon 0.1.2. Three 60-second rounds at 1 Hz, including child/helper processes;
ratios use unrounded values. Existing profiles were used, with no benchmark-driven
UI interaction. Matching views and connected-cluster workloads were not verified,
so this is an observational baseline—not a UI-speed or live-cluster throughput claim.
Results vary by workload and machine.

[Python benchmark](tools/benchmarks/macos_apps.py) ·
[Results and methodology](docs/benchmarks/macos-idle-2026-11-11.json)

## OS roadmap

| Platform | Current target | Next |
| --- | --- | --- |
| macOS · Apple Silicon | CI + DMG/app archive packaging | Packaged smoke tests, Developer ID signing/notarization |
| Windows · x86 | Compile CI + unsigned portable ZIP | Native smoke tests, signing and installer experience |
| Windows · x64/ARM64 | No release target yet | Dedicated builds and validation |
| macOS · Intel | No release target yet | Build and hardware validation |
| Linux | Rust source; CI/builds currently deferred | Restore platform CI, desktop validation and packages |

Download from [Releases](https://github.com/yan-ad/iceon/releases).
macOS packages are ad-hoc signed, **not notarized**; Windows packages are unsigned.
Install manually—automatic updates are not integrated. See [installation details](docs/releases.md).

## Feature roadmap

| Area | Status | Next |
| --- | --- | --- |
| Native shell, catalog, resource explorer | Implemented | Accessibility, polish and large-cluster performance |
| YAML, logs, terminals, forwarding | Implemented | Broader integration and reconnect/cancellation testing |
| Helm, RBAC and node operations | Core services; native coverage varies | Complete UI workflows and live safety validation |
| Metrics and custom resources | Bounded reads/browsing | Richer observability and custom-resource workflows |
| Secrets | Vault userpass + read-only KV v2 | More providers and authentication methods |
| Extensions | No extension host | Evaluate a native extension model and Freelens workflow portability |
| Distribution | macOS/Windows packaging | Trusted signing, automatic updates and more architectures |

## Develop

Requires Rust stable and the platform's native build tools (Xcode tools on macOS;
MSVC on Windows). No JavaScript toolchain or browser runtime is required.

```sh
cargo run -p iceon-app
cargo test --workspace --all-features --locked
```

Architecture and safety contracts: [DESIGN.md](DESIGN.md). Contributions and
real-world bug reports are welcome; use trusted kubeconfigs and a test cluster first.

## Inspiration & license

Inspired by [Freelens](https://github.com/freelensapp/freelens) and its Kubernetes
exploration workflows, with a separate native Rust/GPUI implementation.
**Freelens extensions cannot currently run in Iceon**; compatibility is research,
not a promised API or feature-parity claim.

Iceon is licensed under the [MIT License](LICENSE). Third-party dependencies and
assets retain their respective licenses and notices.

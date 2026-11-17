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
- **Extensions (Experimental):** all unofficial port of freelens extension is running natively in rust without installing any package

## How fast is it?

An overall resource-usage snapshot of the installed apps, measured with Python:

| Metric                                   | Freelens | Iceon   | #                       |
| ---------------------------------------- | -------- | ------- | ----------------------- |
| CPU Usage (mean, % of one core)          | 1.89%    | 0.16%   | 12.1× lower CPU usage   |
| Memory Usage (median physical footprint) | 633 MiB  | 209 MiB | 3.0× lower memory usage |

Measured on an Apple M4 with 16 GiB RAM, macOS 27.0.1, using Freelens 2.0.0
and Iceon 0.1.2. Three 60-second rounds at 1 Hz, including child/helper processes;
ratios use unrounded values. Existing profiles were used, with no benchmark-driven
UI interaction. Matching views and connected-cluster workloads were not verified,
so this is an observational baseline—not a UI-speed or live-cluster throughput claim.
Results vary by workload and machine.

[Python benchmark](tools/benchmarks/macos_apps.py) ·
[Results and methodology](docs/benchmarks/macos-idle-2026-11-11.json)

## Installation
1. Download from [Releases](https://github.com/yan-ad/iceon/releases).
2. Choose your matched (Mac, Windows, or Linux)

> Current release status is unsigned, so your os might ask security risks, for mac users please run this `xattr -dr com.apple.quarantine /Applications/Iceon.app` manually before launch the apps

## OS roadmap

| Platform              | Current target      | Next                           |
| --------------------- | ------------------- | ------------------------------ |
| macOS · Apple Silicon | Main release target |
| Windows · x86         | Main release target |
| Windows · x64/ARM64   | Planned             | .exe release artifact          |
| macOS · Intel         | Planned             | universal.dmg release artifact |
| Linux                 | Planned             | .flatpak release artifact      |

Download from [Releases](https://github.com/yan-ad/iceon/releases).
macOS packages are ad-hoc signed, **not notarized**; Windows packages lack Authenticode signing.
The `0.1.6` setup/feed pipeline targets older updater-enabled clients; `0.1.2`
retains its ZIP assets. The native app cannot yet update itself. See [installation details](docs/releases.md).

## Feature roadmap

| Area                                     | Status                 | Next                                                                |
| ---------------------------------------- | ---------------------- | ------------------------------------------------------------------- |
| Native shell, catalog, resource explorer | Implemented            | Accessibility, polish and large-cluster performance                 |
| YAML, logs, terminals, forwarding        | Implemented            | Broader integration and reconnect/cancellation testing              |
| Helm, RBAC and node operations           | Implemented            | Complete UI workflows and live safety validation                    |
| Metrics and custom resources             | Implemented            | Richer observability and custom-resource workflows                  |
| Secrets                                  | Implemented            | More providers and authentication methods                           |
| Extensions                               | Experimental, Built-in | Evaluate a native extension model and Freelens workflow portability |
| Distribution                             | Unsigned-package       | Platform signing, native automatic updates and more architectures   |

## Extensions Support

All of this extension is unofficial port based on https://github.com/freelensapp orgs. Since we use rust so we can't have their commit attribution here

| Extension (npm)                     | Ported Version | Status |
| ----------------------------------- | -------------- | ------ |
| @freelensapp/keda-extension         | 2.0.3          | Opt-in |
| @freelensapp/fluxcd-extension       | 2.2.0          | Opt-in |
| @freelensapp/cnpg-extension         | 2.1.0          | Opt-in |
| @freelensapp/gateway-api-extension  | 2.1.0          | Opt-in |
| @freelensapp/eso-extension          | 2.2.0          | Opt-in |
| @freelensapp/kafka-extension        | 2.1.0          | Opt-in |
| @freelensapp/rabbitmq-extension     | 2.0.2          | Opt-in |
| @freelensapp/velero-extension       | 2.2.0          | Opt-in |
| @freelensapp/karpenter-extension    | 2.1.0          | Opt-in |
| @freelensapp/kamaji-extension       | 2.0.1          | Opt-in |
| @freelensapp/sveltos-extension      | 2.0.0          | Opt-in |
| @freelensapp/gpu-extension          | 2.0.1          | Opt-in |
| @freelensapp/vals-extension         | 2.0.1          | Opt-in |
| @freelensapp/resource-map-extension | 2.1.0          | Opt-in |
| @freelensapp/ai-extension           | 2.0.0          | Opt-in |
| @freelensapp/for-claude-extension   | 2.0.0          | Opt-in |
| @freelensapp/agentbridge-extension  | 2.0.0          | Opt-in |
| @freelensapp/kubeswift-extension    | 2.0.0          | Opt-in |

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

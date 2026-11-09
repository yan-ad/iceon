# Iceon (`icekube`) — Pure-Rust GPUI Workspace (`crates/iceon-*`)

Iceon adalah aplikasi desktop Kubernetes & HashiCorp Vault KV v2 yang ditulis **100% dalam Rust murni** menggunakan [GPUI Kit](https://gpui-kit.com/) ([`gpui_kit` v0.9.0](https://docs.rs/gpui-kit/latest/gpui_kit/)), tanpa React, TypeScript/JavaScript, ataupun Tauri, dan diorganisasikan sebagai **Cargo Workspace** di bawah direktori `crates/iceon-*` (tanpa folder `src/` atau `src-tauri/` di root).

## Struktur Workspace (`crates/iceon-*`)

- **`crates/iceon-core`** (`lib` crate `iceon_core`):
  - `src/runtime.rs`: Shared multi-threaded Tokio runtime (`handle()`, `spawn()`, `spawn_blocking()`, `block_on()`) dan resolusi direktori data aplikasi (`app_data_dir()`, `home_dir()`).
  - `src/catalog.rs` & `src/catalog/`: `CatalogEngine` — impor kubeconfig metadata-only (termasuk discovery otomatis `~/.kube/config`), penyimpanan SQLite, serta pin cluster dan section.
  - `src/connection.rs` & `src/connection/`: `ConnectionEngine` — sesi koneksi cluster berbasis lease tunggal, RBAC `SelfSubjectAccessReview`, cache namespace, live watch Pod (`pod_sync`), detail & koleksi resource, aksi workload (`Scale`, `Restart`, `Suspend`, `Resume`, `Trigger`), aksi node (`Cordon`, `Uncordon`, `Drain`), port-forward Pod & Service, integrasi CLI Helm v3.17+/v4, Managed RBAC Users, dan Custom Resources (CRD).
  - `src/secrets/mod.rs`: `secrets::Engine` — integrasi HashiCorp Vault KV v2 dengan penyimpanan kredensial di OS Keyring dan auto-expire reveal 30 detik.
  - `src/preferences.rs` & `src/route_history.rs`: `PreferencesEngine` — penyimpanan preferensi aplikasi, proxy, Kubernetes, editor, terminal, dan riwayat rute di SQLite.
  - `src/tls.rs`, `src/project.rs`, `src/dev_engine.rs`: Inisialisasi penyedia kriptografi `rustls` (`ring`) dan utilitas runtime.
- **`crates/iceon-ui`** (`lib` crate `iceon_ui`):
  - `src/lib.rs`: Shell utama `IceonApp`, custom `TitleBar`, navigasi Back/Forward, Quick Jump (`Search`), dan sidebar kolapsibel.
  - `src/navigation.rs`: Definisi rute, pohon navigasi cluster, 41 definisi tabel `ResourceKind`, dan template YAML pembuatan resource.
  - `src/catalog_view.rs`: Tabel cluster catalog, dialog file native GPUI untuk impor kubeconfig, pin/unpin, dan konfirmasi hapus.
  - `src/cluster_view.rs`: Cluster Overview, Workloads Overview, Resource Table (dengan Live Watch Pod & modal Create YAML), Helm Charts/Releases & Install modal, Port Forwarding, Managed RBAC Users, dan Custom Resources.
  - `src/inspector_view.rs`: Panel kanan resizable untuk inspeksi detail resource, Pod, dan Helm beserta editor YAML (`tree-sitter-yaml`) dengan server dry-run.
  - `src/pod_dock_view.rs`: Panel bawah resizable untuk stream `Logs`, `Shell` (`exec`), `Attach`, dan daftar Port Forward aktif.
  - `src/secrets_view.rs`: Workspace Vault KV v2.
  - `src/preferences_view.rs`: Modal pengaturan 5 tab dengan pergantian tema langsung (`Theme::change`).
- **`crates/iceon-app`** (`bin` crate `iceon-app` / binary `iceon`):
  - `src/main.rs`: Entrypoint binary aplikasi desktop GPUI (`iceon_ui::run()`).
- **`crates/iceon-xtask`** (`bin` crate `iceon-xtask`):
  - `src/main.rs` & `src/verify_update.rs`: Utilitas rilis dan verifikasi tanda tangan update.

## Konfigurasi Manifest Cargo (`Cargo.toml`)

Simpan berkas `Cargo.toml` berikut pada masing-masing direktori workspace sebelum menjalankan `cargo run`:

### 1. Root `/Cargo.toml`
```toml
[workspace]
resolver = "2"
members = [
    "crates/iceon-core",
    "crates/iceon-ui",
    "crates/iceon-app",
    "crates/iceon-xtask",
]
default-members = ["crates/iceon-app"]

[workspace.package]
version = "0.1.8"
edition = "2024"
license = "Apache-2.0"
authors = ["you"]

[workspace.dependencies]
iceon-core = { path = "crates/iceon-core" }
iceon-ui = { path = "crates/iceon-ui" }
gpui-kit = { version = "0.9.0", features = ["tree-sitter-yaml"] }
serde_json = "1.0"
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
log = "0.4"
base64 = "0.22"
bytes = "1"
chrono = { version = "0.4", default-features = false, features = ["clock", "std"] }
dirs = "6"
futures-util = "0.3"
http = "1"
k8s-openapi = { version = "0.27", default-features = false, features = ["latest"] }
keyring = { version = "3.6", default-features = false, features = ["apple-native", "windows-native", "sync-secret-service", "crypto-rust"] }
kube = { version = "3.1", default-features = false, features = ["client", "config", "rustls-tls", "ws", "gzip"] }
percent-encoding = "2.3"
process-wrap = { version = "9.0.1", features = ["tokio1", "kill-on-drop"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
rusqlite = { version = "0.32", features = ["bundled"] }
rustls = { version = "0.23", default-features = false, features = ["ring", "std"] }
semver = "1"
sha2 = "0.10"
tempfile = "3"
thiserror = "2"
tokio = { version = "1", features = ["io-util", "macros", "net", "process", "rt-multi-thread", "sync", "time"] }
tokio-util = "0.7"
url = "2"
uuid = { version = "1", features = ["v4"] }
zeroize = { version = "1.8", features = ["derive"] }
```

### 2. `crates/iceon-core/Cargo.toml`
```toml
[package]
name = "iceon-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
base64.workspace = true
bytes.workspace = true
chrono.workspace = true
dirs.workspace = true
futures-util.workspace = true
http.workspace = true
k8s-openapi.workspace = true
keyring.workspace = true
kube.workspace = true
log.workspace = true
percent-encoding.workspace = true
process-wrap.workspace = true
reqwest.workspace = true
rusqlite.workspace = true
rustls.workspace = true
semver.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_yaml.workspace = true
sha2.workspace = true
tempfile.workspace = true
thiserror.workspace = true
tokio.workspace = true
tokio-util.workspace = true
url.workspace = true
uuid.workspace = true
zeroize.workspace = true

[dev-dependencies]
http-body-util = "0.1"
hyper = { version = "1", features = ["http1", "server"] }
hyper-util = { version = "0.1", features = ["tokio"] }
rcgen = "0.13"
tokio-tungstenite = "0.28"
```

### 3. `crates/iceon-ui/Cargo.toml`
```toml
[package]
name = "iceon-ui"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
gpui-kit.workspace = true
iceon-core.workspace = true
tokio.workspace = true
```

### 4. `crates/iceon-app/Cargo.toml`
```toml
[package]
name = "iceon-app"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "iceon"
path = "src/main.rs"

[dependencies]
iceon-ui.workspace = true
```

## Menjalankan Aplikasi & Unit Tests

```bash
cargo run -p iceon-app
cargo test --workspace
```

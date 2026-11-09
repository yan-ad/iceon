# AGENTS.md — Pure-Rust GPUI Workspace Architecture (`crates/iceon-*`)

Proyek ini menggunakan arsitektur **100% Pure Rust** berbasis `gpui-kit = "0.9.0"` tanpa layer webview (tanpa React, TypeScript/JavaScript, Vite, Bun, atau Tauri) dan disusun dalam Cargo Workspace di bawah `crates/iceon-*` (tanpa direktori `src/` atau `src-tauri/` di root).

## Struktur Workspace (`crates/`)

- `crates/iceon-core`: Engine domain murni Rust (`catalog`, `connection` beserta 27 submodulnya, `preferences`, `route_history`, `secrets`, `tls`, `runtime`, `project`, dan `dev_engine`).
- `crates/iceon-ui`: Komponen dan view GPUI (`lib.rs`, `navigation.rs`, `catalog_view.rs`, `cluster_view.rs`, `inspector_view.rs`, `pod_dock_view.rs`, `secrets_view.rs`, `preferences_view.rs`).
- `crates/iceon-app`: Crate binary utama (`src/main.rs`) yang memanggil `iceon_ui::run()`.
- `crates/iceon-xtask`: Crate utilitas rilis dan verifikasi tanda tangan update (`cargo xtask`).

## Invarian Keamanan & Arsitektur

1. **Metadata-Only Catalog Import**: `CatalogEngine` hanya membaca nama cluster/context dari kubeconfig dan tidak pernah menyimpan token/key/sertifikat ke SQLite.
2. **Single Lease Workspace**: `ConnectionEngine` membatasi sesi aktif per workspace dan membatalkan seluruh stream/watch (`CancellationToken`) saat sesi ditutup atau diganti.
3. **Server Dry-Run & Preconditions**: Semua mutasi resource (`create`, `resource_edit`, `delete`, `workload_actions`, `node_actions`, `node_drain`, `rbac_management`) memverifikasi `uid` dan `resourceVersion` serta mendukung server-side dry-run.
4. **Redacted Projections**: `Secrets` dan `ConfigMaps` hanya mengekspos metadata aman (`PartialObjectMetadata`); nilai rahasia Vault hanya ditampilkan saat aksi `reveal` eksplisit dengan masa berlaku 30 detik.

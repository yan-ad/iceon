# Credential renewal

## Ownership and supported boundary

`crates/iceon-core/src/connection/config.rs`, `auth.rs`, `renewal.rs` and
`crates/iceon-core/src/connection.rs` own selected-context client setup,
credential resolution and session lifecycle. GPUI views receive typed public
results, never raw credentials. This is not provider certification or parity
with another Kubernetes desktop application.

Only supported configured exec-helper/token-file renewal belongs to this path.
Iceon does not invent interactive cloud login, merge arbitrary account inventories,
or treat the catalog as a credential cache. Kubeconfigs must be trusted because
connecting can execute their selected helper.

## Invariants

- Keep client/credential state bound to the current connection lease and selected identity.
- Use verified HTTPS and configured CA handling; renewal is not an insecure-TLS fallback.
- Bound helper execution, output and network operations; sanitize public errors.
- Cancel pending renewal and reads on disconnect/replacement. Late replies must not install
  a client into a newer session.
- Preserve the last valid state only according to the engine's expiry rules; do not present
  expired credentials as a healthy connection.
- Do not persist helper tokens, private keys or certificates in catalog SQLite.
- No background retry is permission to mutate resources or silently switch identity.

Native acceptance should cover expiry, helper failure, disconnect during renewal
and fresh connection after a failure using temporary configs and disposable helpers.
See [platform discovery](platform-discovery.md) for acquisition/login boundaries.

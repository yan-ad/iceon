# Permission management

Iceon separates permission observations from authorization. Kubernetes is always
the authority; button state or a previously successful dry-run is not a grant.
Core services exist, while native workflow coverage and live-cluster acceptance
remain separate from service/unit-test coverage.

## Source owners

| Owner under `crates/iceon-core/src/connection/` | Responsibility |
| --- | --- |
| `permissions.rs` | Scoped self-access checks and permission observations |
| `rbac.rs` | Bounded observed RBAC subjects |
| `rbac_management.rs` | Managed ServiceAccounts/templates/assignments, reviewed plans and guarded apply |

The native Users/management presentation is in
`crates/iceon-ui/src/cluster_view.rs`; it calls typed Rust APIs directly.

## Safety contract

- Kubernetes has no generic User resource. An observed subject does not establish a login account.
- Managed ServiceAccounts/assignments are constrained workflows, not a general identity provider.
- Plans bind to session and object identity; revalidate permissions/preconditions at apply time.
- Support server dry-run and explicit confirmation; preserve UID/resourceVersion protections.
- Keep unrelated subjects/bindings outside a managed operation's ownership.
- Report partial application and conflicts. Multi-object operations are not atomic transactions.
- Cancel work on session replacement and reject late results from the previous session.
- Do not expose generated credentials, secret values or raw authorization errors in generic metadata.

Full arbitrary RBAC editors, new authentication providers and broader credential
lifecycle UI are not implied by these services. Use a disposable test cluster for
acceptance checks covering denied reads, expired plans, conflicts, partial failure
and cancellation. See [design](../DESIGN.md).

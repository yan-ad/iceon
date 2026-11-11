# Custom resources

## Scope and ownership

Custom-resource browsing separates CRD inventory, usable group/kind discovery and
one selected definition's instance list. It is metadata-first, not a generic editor
for arbitrary resource payloads or a guarantee that every CRD is available.

- Core: `crates/iceon-core/src/connection/custom_resources.rs`.
- Native navigation/loading: `crates/iceon-ui/src/cluster_view.rs`.
- Resource presentation: `crates/iceon-ui/src/resource_table.rs` and `inspector_view.rs`.

Requests carry the selected cluster/session, definition identity and namespace
scope. Discovery is bounded; partial results, unavailable APIs and RBAC denial must
remain explicit. Navigation/refresh cancels obsolete reads and late completions
must not overwrite a new target.

The instance list requests safe Kubernetes metadata projections. It must not fall
back to dumping raw objects when metadata negotiation fails. CRD schema presence
is not permission to reveal secret data, write arbitrary instances or claim full
schema-aware forms. Broader custom-resource editing remains separate work.

Test discovery, namespace scope, denial, partial pages, definition replacement and
cancellation with synthetic fixtures. Native/live-cluster validation remains necessary.
See [design](../DESIGN.md) and [resource inspector](resource-inspector-reference.md).

# Native resource inspector

## Ownership

- `crates/iceon-ui/src/inspector_view.rs`: GPUI resource inspector and native actions.
- `crates/iceon-ui/src/editor_surface.rs`, `yaml_dock.rs`: YAML editing surfaces/tabs.
- `crates/iceon-core/src/connection/details.rs`: named-object reads, identity and cancellation.
- `crates/iceon-core/src/connection/detail_facts.rs`: kind-specific facts.
- `crates/iceon-core/src/connection/detail_metadata.rs`: safe metadata projection.
- `crates/iceon-core/src/connection/detail_collections.rs`: bounded related collections.
- `crates/iceon-core/src/connection/resource_edit.rs`, `create.rs`, `delete.rs`: guarded mutations.

## Contract

The inspector presents typed resource facts, metadata, events and related references.
It is not a raw-object dump or a claim of complete per-kind workflow parity with
Freelens. Related resources must keep the current cluster/namespace identity.

Named reads validate identity and reject stale sessions. Refresh errors, missing
metrics and partial related collections remain explicit rather than becoming empty
success. YAML inspection must preserve sensitive-resource redaction: Secrets and
ConfigMaps do not expose values through generic inspection.

Edits/deletions bind to UID/resourceVersion where applicable, support server dry-run,
and require explicit confirmation. Conflicts require a new read/review, not an
automatic destructive retry. Creation scaffolds are not complete valid manifests.

The native editor uses GPUI Kit's YAML integration. There is no browser editor,
DOM drawer, component test fixture or frontend callback transport. Verify keyboard
focus, scrolling, resize, clipboard and confirmation behavior in the native app;
core projection tests do not certify the window experience.

See [design](../DESIGN.md) and [custom resources](custom-resources.md).

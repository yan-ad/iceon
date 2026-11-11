# Cluster access and provider boundaries

## Current native workflow

Iceon imports trusted kubeconfig files into a metadata-only catalog. Default
and configured source reconciliation is separate from an explicit manual import;
preferences can select files/shallow directories. See [preferences](preferences.md).
The native Catalog lives in `crates/iceon-ui/src/catalog_view.rs`; discovery and
persistence live in `crates/iceon-core/src/catalog.rs` and `catalog/`.

Import does not authenticate, run helpers or contact a cluster. Opening a workspace
uses the selected context and can execute its configured credential helper, so
untrusted kubeconfigs are unsafe. Credentials, private keys, certificates, helper
environment and raw config contents must not enter catalog SQLite or public views.

Client setup and authentication live in
`crates/iceon-core/src/connection/config.rs`, `auth.rs` and `renewal.rs`.
Use verified HTTPS and configured CA trust. Provider labels do not create new
authentication mechanisms, discover accounts or certify live-provider compatibility.
Interactive provider login remains outside Iceon.

## Prepare access outside Iceon

| Platform | Typical prerequisite |
| --- | --- |
| GKE | Exported kubeconfig and installed configured authentication plugin |
| DigitalOcean / Linode / Vultr / Scaleway / Civo | Provider-exported cluster kubeconfig |
| OCI / IBM / Alibaba / Tencent / OVHcloud | Provider-authorized kubeconfig; configured helper where required |
| OpenShift | Existing authorized `oc` login/kubeconfig |
| Rancher / RKE2 / K3s | Authorized reachable cluster context and certificate material |
| MicroK8s / kind / minikube | Running local cluster and host-reachable exported context |
| Self-managed Kubernetes | Administrator-provided trusted kubeconfig and network route |

These are preparation hints, not SDK integrations, cloud inventory adapters or
live authentication test results. Referenced certificate/helper files and
executables must exist on the device running Iceon. Importing metadata is not
proof that networking, credential renewal or RBAC will succeed.

## Future cloud-account discovery

Cloud-account enumeration is not implemented. Any future adapter must own bounded
pagination, fixed/validated HTTPS endpoints, cancellation, explicit account/region
selection and private credential handling. Management credentials are separate from
Kubernetes credentials and must never be stored in catalog metadata. A provider
selector must not pretend to enumerate clusters without a real provider response.

Synthetic tests should cover source identity, metadata redaction, helper isolation,
TLS rejection, cancellation and unavailable sources. Native import/focus/keyboard
behavior and provider login require separately approved acceptance tests, not
removed browser fixtures. See [credential renewal](credential-renewal.md) and
[design](../DESIGN.md).

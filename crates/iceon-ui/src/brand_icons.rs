//! Embedded brand glyphs and boundary-aware Kubernetes API-group presentation.

macro_rules! brand_assets {
    ($($file:literal),+ $(,)?) => {
        pub(crate) const ASSETS: &[(&str, &[u8])] = &[
            $((concat!("icons/custom-brand/", $file, ".svg"),
                include_bytes!(concat!("../assets/icons/custom-brand/", $file, ".svg")))),+
        ];
    };
}
brand_assets!(
    "argoproj",
    "cillium",
    "gke",
    "google",
    "googleapis",
    "grafana",
    "k8s",
    "keda",
    "traefik",
    "velero",
    "nginx",
    "external-secrets",
    "calico",
    "metallb",
    "kubernetes",
);

pub(crate) const KUBERNETES_PATH: &str = "icons/custom-brand/k8s.svg";
pub(crate) const CLUSTER_PATH: &str = "icons/custom-brand/kubernetes.svg";

/// A domain is either exact or a dot-delimited suffix, never a brand substring.
/// More specific Google domains precede their parent domains. Unknown groups use
/// the generic Kubernetes glyph; discovery data is never treated as an asset path.
pub(crate) fn api_group_path(group: &str) -> &'static str {
    const DOMAINS: &[(&str, &str)] = &[
        ("argoproj.io", "icons/custom-brand/argoproj.svg"),
        // Keep the spelling of the supplied asset, not the project's domain.
        ("cilium.io", "icons/custom-brand/cillium.svg"),
        ("gke.io", "icons/custom-brand/gke.svg"),
        ("gke.google.com", "icons/custom-brand/gke.svg"),
        ("googleapis.com", "icons/custom-brand/googleapis.svg"),
        ("google.com", "icons/custom-brand/google.svg"),
        ("grafana.com", "icons/custom-brand/grafana.svg"),
        ("grafana.org", "icons/custom-brand/grafana.svg"),
        ("grafana.integreatly.org", "icons/custom-brand/grafana.svg"),
        ("keda.sh", "icons/custom-brand/keda.svg"),
        ("traefik.io", "icons/custom-brand/traefik.svg"),
        ("traefik.containo.us", "icons/custom-brand/traefik.svg"),
        ("velero.io", "icons/custom-brand/velero.svg"),
        ("nginx.org", "icons/custom-brand/nginx.svg"),
        ("nginx.com", "icons/custom-brand/nginx.svg"),
        ("projectcalico.org", "icons/custom-brand/calico.svg"),
        ("tigera.io", "icons/custom-brand/calico.svg"),
        ("metallb.io", "icons/custom-brand/metallb.svg"),
        ("metallb.universe.tf", "icons/custom-brand/metallb.svg"),
        (
            "external-secrets.io",
            "icons/custom-brand/external-secrets.svg",
        ),
        ("k8s.io", KUBERNETES_PATH),
        ("kubernetes.io", KUBERNETES_PATH),
    ];
    DOMAINS
        .iter()
        .find(|(domain, _)| {
            group == *domain
                || group
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1)
        })
        .map_or(KUBERNETES_PATH, |(_, path)| *path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_group_domains_and_aliases_map_to_supplied_brands() {
        for (domain, brand) in [
            ("argoproj.io", "argoproj"),
            ("cilium.io", "cillium"),
            ("gke.io", "gke"),
            ("gke.google.com", "gke"),
            ("google.com", "google"),
            ("cloud.google.com", "google"),
            ("googleapis.com", "googleapis"),
            ("grafana.com", "grafana"),
            ("grafana.org", "grafana"),
            ("grafana.integreatly.org", "grafana"),
            ("keda.sh", "keda"),
            ("traefik.io", "traefik"),
            ("traefik.containo.us", "traefik"),
            ("velero.io", "velero"),
            ("nginx.org", "nginx"),
            ("nginx.com", "nginx"),
            ("external-secrets.io", "external-secrets"),
            ("projectcalico.org", "calico"),
            ("tigera.io", "calico"),
            ("metallb.io", "metallb"),
            ("metallb.universe.tf", "metallb"),
            ("k8s.io", "k8s"),
            ("kubernetes.io", "k8s"),
        ] {
            let expected = format!("icons/custom-brand/{brand}.svg");
            assert_eq!(api_group_path(domain), expected, "{domain}");
            assert_eq!(api_group_path(&format!("networking.{domain}")), expected);
            // A near miss under google.com still legitimately matches Google,
            // but must not inherit the more-specific GKE brand.
            let prefixed = format!("not{domain}");
            assert_eq!(
                api_group_path(&prefixed),
                if domain.ends_with(".google.com") {
                    "icons/custom-brand/google.svg"
                } else {
                    KUBERNETES_PATH
                },
                "{prefixed}"
            );
            let unrelated = format!("{domain}.example.org");
            assert_eq!(api_group_path(&unrelated), KUBERNETES_PATH, "{unrelated}");
        }
    }

    #[test]
    fn unrelated_and_untrusted_groups_have_a_fixed_generic_fallback() {
        for group in [
            "",
            "example.io",
            "argoproj",
            "cillium.io",
            "my-grafana.example.org",
            "googleapis.example.io",
            "evilgoogle.com",
            ".argoproj.io",
            "argoproj.io/../../velero.svg",
            "icons/custom-brand/argoproj.svg",
        ] {
            assert_eq!(api_group_path(group), KUBERNETES_PATH, "{group}");
        }
    }
}

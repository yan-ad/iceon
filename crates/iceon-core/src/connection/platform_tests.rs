use super::{ConnectionError, config};
use crate::catalog::{
    CatalogEngine, CatalogSnapshot, ImportRequest, PinRequest, SectionPinRequest,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Clone, Copy)]
enum Auth {
    Token,
    Certificate,
    Exec(&'static str, &'static [&'static str]),
}

struct Platform {
    name: &'static str,
    auth: Auth,
    server: &'static str,
}

fn platforms() -> Vec<Platform> {
    use Auth::*;
    let cases = [
        ("do-nyc1-production", Token),
        ("lke12345-ctx", Token),
        ("vke-01234567-89ab-cdef", Certificate),
        (
            "context-oke-synthetic",
            Exec(
                "oci",
                &[
                    "ce",
                    "cluster",
                    "generate-token",
                    "--cluster-id",
                    "ocid1.cluster.synthetic",
                ],
            ),
        ),
        (
            "ibm-production/bx2",
            Exec(
                "ibmcloud",
                &["ks", "cluster", "config", "--cluster", "synthetic-cluster"],
            ),
        ),
        ("ack-production-admin", Certificate),
        ("cls-tke-synthetic", Certificate),
        ("scw-k8s-fr-par-production", Token),
        ("ovh-production", Certificate),
        ("civo-production-token", Token),
        ("civo-production-certificate", Certificate),
        ("hetzner-k3s", Certificate),
        ("production/api-openshift-example:6443/developer", Token),
        ("rancher-production", Token),
        ("rancher-production-direct", Certificate),
        ("rke2-admin@production", Certificate),
        ("default-k3s", Certificate),
        ("microk8s", Certificate),
        ("kind-kind", Certificate),
        ("minikube", Certificate),
        (
            "gke_synthetic-project_us-central1_production",
            Exec("gke-gcloud-auth-plugin", &[]),
        ),
        ("my team's arbitrary workspace", Token),
    ];
    cases
        .into_iter()
        .map(|(name, auth)| Platform {
            name,
            auth,
            server: if name == "rancher-production" {
                "https://rancher.example.invalid/k8s/clusters/c-synthetic"
            } else {
                "https://api.example.invalid:6443"
            },
        })
        .collect()
}

fn user(auth: Auth, index: usize) -> Value {
    match auth {
        Auth::Token => json!({"token": format!("SYNTHETIC_BEARER_SECRET_{index}")}),
        // Preparation preserves opaque bytes; these are deliberately not usable TLS credentials.
        Auth::Certificate => json!({
            "client-certificate-data": STANDARD.encode(format!("SYNTHETIC_CERTIFICATE_SECRET_{index}")),
            "client-key-data": STANDARD.encode(format!("SYNTHETIC_PRIVATE_KEY_SECRET_{index}")),
        }),
        Auth::Exec(command, args) => json!({"exec": {
            "apiVersion": "client.authentication.k8s.io/v1beta1",
            "command": command,
            "args": args,
            "interactiveMode": "Never",
            "provideClusterInfo": false,
            "env": [{"name": "SYNTHETIC_AUTH_INPUT", "value": format!("SYNTHETIC_EXEC_SECRET_{index}")}],
        }}),
    }
}

fn document(cases: &[Platform]) -> Value {
    json!({
        "apiVersion": "v1", "kind": "Config",
        "current-context": cases[0].name,
        "clusters": cases.iter().enumerate().map(|(i, case)| json!({
            "name": format!("endpoint-{i}"), "cluster": {
                "server": case.server,
                "certificate-authority-data": STANDARD.encode(format!("SYNTHETIC_CA_SECRET_{i}")),
            }
        })).collect::<Vec<_>>(),
        "contexts": cases.iter().enumerate().map(|(i, case)| json!({
            "name": case.name, "context": {
                "cluster": format!("endpoint-{i}"), "user": format!("credential-user-{i}"), "namespace": "workloads"
            }
        })).collect::<Vec<_>>(),
        "users": cases.iter().enumerate().map(|(i, case)| json!({
            "name": format!("credential-user-{i}"), "user": user(case.auth, i)
        })).collect::<Vec<_>>()
    })
}

fn write(path: &Path, document: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(document).unwrap()).unwrap();
}

fn import(engine: &CatalogEngine, path: &Path) -> CatalogSnapshot {
    engine
        .import_kubeconfig(ImportRequest {
            path: path.to_str().unwrap().to_owned(),
        })
        .unwrap()
}

fn assert_metadata_only(snapshot: &CatalogSnapshot, directory: &Path, document: &Value) {
    let serialized = serde_json::to_vec(snapshot).unwrap();
    let mut persisted = Vec::new();
    // Include SQLite journals/sidecars, not just the currently live rows.
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            persisted.extend(fs::read(path).unwrap());
        }
    }
    assert!(directory.join("catalog.sqlite3").is_file());
    let mut private = vec![
        "api.example.invalid".to_owned(),
        "rancher.example.invalid".to_owned(),
    ];
    for entry in document["users"].as_array().unwrap() {
        private.push(entry["name"].as_str().unwrap().to_owned());
        // Credential values, not generic exec schema strings such as "Never".
        for field in ["token", "client-certificate-data", "client-key-data"] {
            if let Some(value) = entry["user"][field].as_str() {
                private.push(value.to_owned());
            }
        }
        if let Some(env) = entry["user"]["exec"]["env"].as_array() {
            for variable in env {
                private.push(variable["value"].as_str().unwrap().to_owned());
            }
        }
        for field in ["client-certificate-data", "client-key-data"] {
            if let Some(value) = entry["user"][field].as_str() {
                private.push(String::from_utf8(STANDARD.decode(value).unwrap()).unwrap());
            }
        }
    }
    for cluster in document["clusters"].as_array().unwrap() {
        private.push(
            cluster["cluster"]["certificate-authority-data"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    for secret in private {
        for bytes in [&serialized, &persisted] {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "catalog persisted credential material"
            );
        }
    }
}

#[test]
fn platform_tests_default_discovery_and_explicit_import_prepare_every_context() {
    let cases = platforms();
    let input = document(&cases);
    for discovery in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("synthetic-home");
        let path = if discovery {
            home.join(".kube/config")
        } else {
            root.path().join("selected.json")
        };
        write(&path, &input);
        let directory = root.path().join("catalog");
        let engine = CatalogEngine::new_with_home(directory.clone(), Some(home));
        let snapshot = if discovery {
            engine.list().unwrap()
        } else {
            import(&engine, &path)
        };
        assert_eq!(snapshot.discovery_error, None);
        assert_eq!(snapshot.clusters.len(), cases.len());
        assert_eq!(
            snapshot
                .clusters
                .iter()
                .map(|c| c.name.as_str())
                .collect::<BTreeSet<_>>(),
            cases.iter().map(|c| c.name).collect()
        );
        for (i, case) in cases.iter().enumerate() {
            let row = snapshot
                .clusters
                .iter()
                .find(|c| c.name == case.name)
                .unwrap();
            assert_eq!(row.context_name, case.name);
            assert_eq!(row.status, "disconnected");
            assert_eq!(row.source, "kubeconfig");
            if discovery {
                assert_eq!(row.labels, ["file=~/.kube/config"]);
            } else {
                assert_eq!(row.labels, [format!("file={}", path.display())]);
            }
            let target = engine.connection_target(&row.id).unwrap();
            let prepared =
                config::prepare(&target).unwrap_or_else(|error| panic!("{}: {error:?}", case.name));
            assert_eq!(prepared.current_context.as_deref(), Some(case.name));
            assert_eq!(prepared.contexts.len(), 1);
            assert_eq!(prepared.contexts[0].name, case.name);
            assert_eq!(
                prepared.contexts[0]
                    .context
                    .as_ref()
                    .unwrap()
                    .namespace
                    .as_deref(),
                Some("workloads")
            );
            assert_eq!(prepared.clusters.len(), 1);
            assert_eq!(
                prepared.clusters[0]
                    .cluster
                    .as_ref()
                    .unwrap()
                    .server
                    .as_deref(),
                Some(case.server)
            );
            assert_eq!(prepared.auth_infos.len(), 1);
            let actual =
                serde_json::to_value(prepared.auth_infos[0].auth_info.as_ref().unwrap()).unwrap();
            for (field, expected) in user(case.auth, i).as_object().unwrap() {
                assert_eq!(&actual[field], expected, "{}: {field}", case.name);
            }
        }
        assert_metadata_only(&snapshot, &directory, &input);
    }
}

#[test]
fn platform_tests_identical_names_remain_source_scoped_and_reimport_keeps_pins_colors() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("catalog");
    let engine = CatalogEngine::new(directory.clone());
    let input = document(&platforms());
    let first_path = root.path().join("first.json");
    let second_path = root.path().join("second.json");
    write(&first_path, &input);
    write(&second_path, &input);
    let first = import(&engine, &first_path);
    let combined = import(&engine, &second_path);
    assert_eq!(combined.clusters.len(), first.clusters.len() * 2);
    for row in &first.clusters {
        let copies = combined
            .clusters
            .iter()
            .filter(|c| c.name == row.name)
            .collect::<Vec<_>>();
        assert_eq!(copies.len(), 2);
        assert_ne!(copies[0].id, copies[1].id);
        assert_ne!(
            engine.connection_target(&copies[0].id).unwrap().source_path,
            engine.connection_target(&copies[1].id).unwrap().source_path
        );
    }
    let id = first.clusters[0].id.clone();
    engine
        .set_pin(PinRequest {
            id: id.clone(),
            pinned: true,
        })
        .unwrap();
    let pinned = engine
        .set_section_pin(SectionPinRequest {
            cluster_id: id,
            section: "nodes".into(),
            pinned: true,
        })
        .unwrap();
    let mut rotated = input.clone();
    rotated["users"][0]["user"]["token"] = json!("SYNTHETIC_ROTATED_SECRET");
    write(&first_path, &rotated);
    assert_eq!(import(&engine, &first_path), pinned);
    assert_eq!(import(&engine, &second_path), pinned);
    assert_eq!(
        CatalogEngine::new(directory.clone()).list().unwrap(),
        pinned
    );
    assert_metadata_only(&pinned, &directory, &rotated);
}

#[test]
fn platform_tests_selection_discards_unselected_helpers_and_missing_credential_files() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("must-not-exist");
    let mut input = document(&platforms());
    input["current-context"] = json!(platforms()[1].name);
    let users = input["users"].as_array_mut().unwrap();
    users[1]["user"] = json!({"exec": {
        "apiVersion": "client.authentication.k8s.io/v1beta1", "command": "/bin/sh",
        "args": ["-c", format!("touch '{}'", marker.display())], "interactiveMode": "Never"
    }});
    users[2]["user"] = json!({"exec": {"command": root.path().join("missing-provider-cli").to_str().unwrap()},
        "client-certificate": "missing-cert.pem", "client-key": "missing-key.pem", "tokenFile": "missing-token"});
    let path = root.path().join("selection.json");
    write(&path, &input);
    let engine = CatalogEngine::new(root.path().join("catalog"));
    let snapshot = import(&engine, &path);
    let selected = snapshot
        .clusters
        .iter()
        .find(|c| c.name == platforms()[0].name)
        .unwrap();
    let prepared = config::prepare(&engine.connection_target(&selected.id).unwrap()).unwrap();
    assert_eq!(prepared.auth_infos.len(), 1);
    assert_eq!(prepared.auth_infos[0].name, "credential-user-0");
    assert!(
        prepared.auth_infos[0]
            .auth_info
            .as_ref()
            .unwrap()
            .exec
            .is_none()
    );
    assert_eq!(prepared.contexts.len(), 1);
    assert!(!marker.exists());
    assert_eq!(engine.list().unwrap().clusters.len(), platforms().len());
}

#[test]
fn platform_tests_insecure_servers_tls_bypass_and_proxy_urls_are_rejected_at_preparation() {
    let cases = platforms();
    for patch in [
        json!({"server": "http://api.example.invalid"}),
        json!({"server": "https://user:password@api.example.invalid"}),
        json!({"server": "https://api.example.invalid?token=secret"}),
        json!({"server": "https://api.example.invalid#fragment"}),
        json!({"insecure-skip-tls-verify": true}),
        json!({"proxy-url": "https://proxy.example.invalid"}),
        json!({"proxy-url": "http://proxy.example.invalid"}),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut input = document(&cases[..1]);
        input["clusters"][0]["cluster"]
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let path = root.path().join("insecure.json");
        write(&path, &input);
        let engine = CatalogEngine::new(root.path().join("catalog"));
        let snapshot = import(&engine, &path);
        assert_eq!(snapshot.clusters[0].status, "disconnected");
        let target = engine.connection_target(&snapshot.clusters[0].id).unwrap();
        assert!(
            matches!(
                config::prepare(&target),
                Err(ConnectionError::InsecureTransport)
            ),
            "patch: {patch}"
        );
    }
}

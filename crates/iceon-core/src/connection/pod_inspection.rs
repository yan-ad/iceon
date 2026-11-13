//! Deliberately typed, bounded allowlist. Never retain arbitrary Kubernetes JSON.
use super::{ConnectionError, Result, array};
use serde::Serialize;
use serde_json::Value;

const ITEMS: usize = 64;
const TOTAL_ITEMS: usize = 512;
const TEXT: usize = 256;
const TOTAL_TEXT: usize = 32 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodInspection {
    pub termination_grace_period_seconds: Option<u64>,
    pub node_selectors: Vec<PodNodeSelector>,
    pub tolerations: Vec<PodToleration>,
    pub containers: Vec<PodContainerInspection>,
    pub volumes: Vec<PodVolume>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PodNodeSelector {
    pub key: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodToleration {
    pub key: Option<String>,
    pub operator: &'static str,
    pub value: Option<String>,
    pub effect: Option<&'static str>,
    pub toleration_seconds: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodContainerInspection {
    pub name: String,
    pub kind: &'static str,
    pub requests: Vec<PodResourceQuantity>,
    pub limits: Vec<PodResourceQuantity>,
    pub environment: Vec<PodEnvironment>,
    pub environment_from: Vec<PodEnvironmentFrom>,
    pub mounts: Vec<PodMount>,
    pub probes: Vec<PodProbe>,
}
#[derive(Debug, Clone, Serialize)]
pub struct PodResourceQuantity {
    pub name: String,
    pub quantity: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct PodEnvironment {
    pub name: String,
    pub source: PodEnvironmentSource,
}
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PodEnvironmentSource {
    Redacted,
    SecretKeyRef {
        name: Option<String>,
        key: Option<String>,
        optional: Option<bool>,
    },
    ConfigMapKeyRef {
        name: Option<String>,
        key: Option<String>,
        optional: Option<bool>,
    },
    FieldRef {
        field: Option<&'static str>,
    },
    ResourceFieldRef {
        container_name: Option<String>,
        resource: Option<String>,
    },
    Unknown,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodEnvironmentFrom {
    pub prefix: Option<String>,
    pub kind: &'static str,
    pub name: Option<String>,
    pub optional: Option<bool>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodMount {
    pub name: Option<String>,
    pub mount_path: Option<String>,
    pub read_only: bool,
}
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PodProbeKind {
    Http,
    Tcp,
    Exec,
    Grpc,
    Unknown,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodProbe {
    pub purpose: &'static str,
    pub kind: PodProbeKind,
    pub initial_delay_seconds: Option<u64>,
    pub timeout_seconds: Option<u64>,
    pub period_seconds: Option<u64>,
    pub success_threshold: Option<u64>,
    pub failure_threshold: Option<u64>,
    pub termination_grace_period_seconds: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodVolume {
    pub name: Option<String>,
    pub sources: Vec<PodVolumeSource>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodVolumeSource {
    pub kind: &'static str,
    pub name: Option<String>,
    pub optional: Option<bool>,
    pub default_mode: Option<u32>,
    pub expiration_seconds: Option<u64>,
    pub path: Option<String>,
}

#[derive(Default)]
struct Budget {
    items: usize,
    text: usize,
}
impl Budget {
    fn count(&mut self, count: usize) -> Result<()> {
        self.items = self.items.saturating_add(count);
        if count > ITEMS || self.items > TOTAL_ITEMS {
            return Err(ConnectionError::LimitExceeded);
        }
        Ok(())
    }
    fn text(&mut self, value: Option<&str>, path: bool) -> Result<Option<String>> {
        let Some(value) = value else { return Ok(None) };
        self.text = self.text.saturating_add(value.len());
        if value.len() > TEXT || self.text > TOTAL_TEXT {
            return Err(ConnectionError::LimitExceeded);
        }
        let valid = if path {
            !value.is_empty()
                && !value.split('/').any(|part| part == "..")
                && !value.contains(['?', '#', '@', '\\'])
                && !value.chars().any(char::is_control)
        } else {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_. /".contains(&b))
                && !value.contains(' ')
        };
        Ok(valid.then(|| value.to_owned()))
    }
    fn field(&mut self, v: &Value, key: &str) -> Result<Option<String>> {
        self.text(v[key].as_str(), false)
    }
}

pub(super) fn quantities(v: &Value) -> Vec<PodResourceQuantity> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            let quantity = value.as_str()?;
            (name.len() <= TEXT
                && !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_. /".contains(&b))
                && !name.contains(' ')
                && !quantity.is_empty()
                && quantity.len() <= 64
                && quantity.bytes().any(|b| b.is_ascii_digit())
                && quantity
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b".+-eEinumkKMGTPE".contains(&b)))
            .then(|| PodResourceQuantity {
                name: name.clone(),
                quantity: quantity.into(),
            })
        })
        .collect()
}
fn field_ref(v: &Value) -> Option<&'static str> {
    // Never forward arbitrary field paths, especially annotation/label lookups.
    [
        "metadata.name",
        "metadata.namespace",
        "metadata.uid",
        "spec.nodeName",
        "spec.serviceAccountName",
        "status.hostIP",
        "status.podIP",
        "status.podIPs",
    ]
    .into_iter()
    .find(|field| v.as_str() == Some(*field))
}
fn volume_source(
    v: &Value,
    kind: &'static str,
    name_key: &str,
    b: &mut Budget,
) -> Result<PodVolumeSource> {
    Ok(PodVolumeSource {
        kind,
        name: b.field(v, name_key)?,
        optional: v["optional"].as_bool(),
        default_mode: v["defaultMode"]
            .as_u64()
            .filter(|n| *n <= 0o777)
            .map(|n| n as u32),
        expiration_seconds: v["expirationSeconds"].as_u64(),
        path: b.text(v["path"].as_str(), true)?,
    })
}

pub(super) fn project(pod: &Value) -> Result<PodInspection> {
    let mut b = Budget::default();
    let spec = &pod["spec"];
    let mut result = PodInspection {
        termination_grace_period_seconds: spec["terminationGracePeriodSeconds"].as_u64(),
        ..Default::default()
    };
    if let Some(selectors) = spec["nodeSelector"].as_object() {
        b.count(selectors.len())?;
        for (key, value) in selectors {
            if let (Some(key), Some(value)) =
                (b.text(Some(key), false)?, b.text(value.as_str(), false)?)
            {
                result.node_selectors.push(PodNodeSelector { key, value });
            }
        }
    }
    b.count(array(spec, "/tolerations").len())?;
    for v in array(spec, "/tolerations") {
        result.tolerations.push(PodToleration {
            key: b.field(v, "key")?,
            value: b.field(v, "value")?,
            operator: if v["operator"].as_str() == Some("Exists") {
                "Exists"
            } else {
                "Equal"
            },
            effect: ["NoSchedule", "PreferNoSchedule", "NoExecute"]
                .into_iter()
                .find(|s| v["effect"].as_str() == Some(*s)),
            toleration_seconds: v["tolerationSeconds"].as_u64(),
        });
    }
    for (key, kind) in [
        ("containers", "container"),
        ("initContainers", "init"),
        ("ephemeralContainers", "ephemeral"),
    ] {
        b.count(array(spec, &format!("/{key}")).len())?;
        for v in array(spec, &format!("/{key}")) {
            let mut c = PodContainerInspection {
                name: b.field(v, "name")?.unwrap_or_default(),
                kind,
                requests: Vec::new(),
                limits: Vec::new(),
                environment: Vec::new(),
                environment_from: Vec::new(),
                mounts: Vec::new(),
                probes: Vec::new(),
            };
            for (key, target) in [("requests", &mut c.requests), ("limits", &mut c.limits)] {
                let map = &v["resources"][key];
                b.count(map.as_object().map_or(0, |m| m.len()))?;
                for (name, value) in map.as_object().into_iter().flatten() {
                    b.text(Some(name), false)?;
                    b.text(value.as_str(), false)?;
                }
                *target = quantities(map);
            }
            b.count(array(v, "/env").len())?;
            for e in array(v, "/env") {
                let Some(name) = b.field(e, "name")? else {
                    continue;
                };
                let f = &e["valueFrom"];
                let source = if e.get("value").is_some() {
                    PodEnvironmentSource::Redacted
                } else if let Some(r) = f.get("secretKeyRef") {
                    PodEnvironmentSource::SecretKeyRef {
                        name: b.field(r, "name")?,
                        key: b.field(r, "key")?,
                        optional: r["optional"].as_bool(),
                    }
                } else if let Some(r) = f.get("configMapKeyRef") {
                    PodEnvironmentSource::ConfigMapKeyRef {
                        name: b.field(r, "name")?,
                        key: b.field(r, "key")?,
                        optional: r["optional"].as_bool(),
                    }
                } else if let Some(r) = f.get("fieldRef") {
                    PodEnvironmentSource::FieldRef {
                        field: field_ref(&r["fieldPath"]),
                    }
                } else if let Some(r) = f.get("resourceFieldRef") {
                    PodEnvironmentSource::ResourceFieldRef {
                        container_name: b.field(r, "containerName")?,
                        resource: b.field(r, "resource")?,
                    }
                } else {
                    PodEnvironmentSource::Unknown
                };
                c.environment.push(PodEnvironment { name, source });
            }
            b.count(array(v, "/envFrom").len())?;
            for e in array(v, "/envFrom") {
                for (key, kind) in [("secretRef", "secret"), ("configMapRef", "configMap")] {
                    if let Some(r) = e.get(key) {
                        c.environment_from.push(PodEnvironmentFrom {
                            prefix: b.field(e, "prefix")?,
                            kind,
                            name: b.field(r, "name")?,
                            optional: r["optional"].as_bool(),
                        });
                        break;
                    }
                }
            }
            b.count(array(v, "/volumeMounts").len())?;
            for m in array(v, "/volumeMounts") {
                c.mounts.push(PodMount {
                    name: b.field(m, "name")?,
                    mount_path: b.text(m["mountPath"].as_str(), true)?,
                    read_only: m["readOnly"].as_bool().unwrap_or(false),
                });
            }
            for (key, purpose) in [
                ("livenessProbe", "liveness"),
                ("readinessProbe", "readiness"),
                ("startupProbe", "startup"),
            ] {
                if let Some(p) = v.get(key) {
                    b.count(1)?;
                    let kind = if p.get("httpGet").is_some() {
                        PodProbeKind::Http
                    } else if p.get("tcpSocket").is_some() {
                        PodProbeKind::Tcp
                    } else if p.get("exec").is_some() {
                        PodProbeKind::Exec
                    } else if p.get("grpc").is_some() {
                        PodProbeKind::Grpc
                    } else {
                        PodProbeKind::Unknown
                    };
                    c.probes.push(PodProbe {
                        purpose,
                        kind,
                        initial_delay_seconds: p["initialDelaySeconds"].as_u64(),
                        timeout_seconds: p["timeoutSeconds"].as_u64(),
                        period_seconds: p["periodSeconds"].as_u64(),
                        success_threshold: p["successThreshold"].as_u64(),
                        failure_threshold: p["failureThreshold"].as_u64(),
                        termination_grace_period_seconds: p["terminationGracePeriodSeconds"]
                            .as_u64(),
                    });
                }
            }
            result.containers.push(c);
        }
    }
    b.count(array(spec, "/volumes").len())?;
    for v in array(spec, "/volumes") {
        let mut volume = PodVolume {
            name: b.field(v, "name")?,
            sources: Vec::new(),
        };
        for (key, name) in [
            ("secret", "secretName"),
            ("configMap", "name"),
            ("persistentVolumeClaim", "claimName"),
            ("emptyDir", ""),
            ("downwardAPI", ""),
        ] {
            if let Some(s) = v.get(key) {
                b.count(1)?;
                volume.sources.push(volume_source(s, key, name, &mut b)?);
            }
        }
        if let Some(p) = v.get("projected") {
            b.count(1)?;
            volume
                .sources
                .push(volume_source(p, "projected", "", &mut b)?);
            b.count(array(p, "/sources").len())?;
            for s in array(p, "/sources") {
                for key in [
                    "secret",
                    "configMap",
                    "serviceAccountToken",
                    "downwardAPI",
                    "clusterTrustBundle",
                    "podCertificate",
                ] {
                    if let Some(s) = s.get(key) {
                        volume.sources.push(volume_source(s, key, "name", &mut b)?);
                        break;
                    }
                }
            }
        }
        result.volumes.push(volume);
    }
    Ok(result)
}

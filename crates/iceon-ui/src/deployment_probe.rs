//! Fixture-only native Deployment/DaemonSet inspector and real ResourceTable audit.
//! No startup worker, TLS, user home, kubeconfig, connection, or shell is used.
//! `--preview` holds the verified window for twelve seconds; `--light` and
//! `--compact` exercise the shared theme path and narrow layout respectively.
//! `--daemon-set` uses a zero-count DaemonSet with complete empty Pods/Events;
//! `--populated` adds a truthful scheduled/pod-status snapshot (`--empty` wins).
//! Inspector bounds/wheel routing and table scroll are measured independently:
//! without test-support we do not claim inspector text/accessibility assertions
//! or direct observation of its private scrollbar/header bounds.

use crate::{
    IceonApp,
    inspector_view::InspectorTarget,
    resource_scroll::ResourceScrollScope,
    resource_table::{ResourceTable, TableData},
};
use gpui_kit::{component::theme::ThemeMode, prelude::*, *};
use iceon_core::{
    catalog::CatalogEngine,
    connection::{
        ConnectionEngine, ResourceItem, ResourceKind, ResourcePage,
        detail_collections::{CollectionItem, CollectionKind, CollectionState, ResourceCollection},
        detail_facts::{DetailFact, DetailSection},
        detail_metadata::{DetailMetadata, ManagedField, MetadataEntry},
        details::{ResourceDetail, ResourceReference},
        workload_actions::WorkloadAction,
    },
    preferences::PreferencesEngine,
    secrets,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
const ROWS: usize = 40;
const CREATED: &str = "2026-11-10T12:00:00Z";
struct FixtureDirectory(PathBuf);
impl FixtureDirectory {
    fn new() -> Self {
        for nonce in 0u64.. {
            let path = std::env::temp_dir().join(format!(
                "iceon-deployment-probe-{}-{nonce}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create isolated fixture directory: {error}"),
            }
        }
        unreachable!()
    }
}
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn columns(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
fn reference(kind: ResourceKind, name: &str) -> ResourceReference {
    ResourceReference {
        kind,
        name: name.into(),
        namespace: Some("offline-demo".into()),
        uid: Some(format!("fixture-{name}")),
    }
}
fn fixture() -> (ResourceDetail, Vec<ResourceCollection>) {
    let reference = reference(ResourceKind::Deployments, "checkout-api");
    let section = |title: &str, values: &[(&str, &str)]| DetailSection {
        title: title.into(),
        facts: values
            .iter()
            .map(|(label, value)| DetailFact {
                label: label.to_string(),
                value: value.to_string(),
            })
            .collect(),
    };
    let detail = ResourceDetail {
        cluster_id: "offline-deployment-probe".into(),
        session_id: "fixture-only".into(),
        read_id: 1,
        reference: reference.clone(),
        item: ResourceItem {
            name: reference.name.clone(),
            uid: reference.uid.clone(),
            resource_version: Some("42".into()),
            namespace: reference.namespace.clone(),
            status: "Available".into(),
            created_at: Some(CREATED.into()),
            columns: columns(&[
                ("desired", "3"),
                ("ready", "3"),
                ("replicas", "3"),
                ("updated", "3"),
                ("available", "3"),
                ("conditions", "Available,Progressing"),
            ]),
            node: None,
            containers: vec![],
        },
        related: BTreeMap::new(),
        event_diagnostic: None,
        sections: vec![
            section(
                "Replicas",
                &[
                    ("Desired replicas", "3"),
                    ("Ready replicas", "3"),
                    ("Available replicas", "3"),
                ],
            ),
            section(
                "Deployment rollout",
                &[
                    ("Strategy", "RollingUpdate"),
                    ("Unavailable replicas", "0"),
                    ("Max surge", "25%"),
                    ("Max unavailable", "25%"),
                ],
            ),
            section("Selector", &[("app", "checkout-api"), ("tier", "backend")]),
            section(
                "Conditions",
                &[
                    ("Available", "True: MinimumReplicasAvailable"),
                    ("Progressing", "True: NewReplicaSetAvailable"),
                ],
            ),
        ],
        metadata: Some(DetailMetadata {
            labels: vec![
                MetadataEntry {
                    key: "app.kubernetes.io/name".into(),
                    value: "checkout-api".into(),
                },
                MetadataEntry {
                    key: "tier".into(),
                    value: "backend".into(),
                },
            ],
            annotations: vec![
                MetadataEntry {
                    key: "deployment.kubernetes.io/revision".into(),
                    value: "12".into(),
                },
                MetadataEntry {
                    key: "example.test/description".into(),
                    value: "Offline rendering fixture; no cluster is connected".into(),
                },
            ],
            finalizers: vec![],
            deletion_timestamp: None,
            partial: false,
            managed_fields: vec![ManagedField {
                manager: "offline-controller".into(),
                operation: "Update".into(),
                api_version: Some("apps/v1".into()),
                time: Some(CREATED.into()),
                subresource: Some("status".into()),
                fields: "[\"status.availableReplicas\",\"status.conditions\"]".into(),
            }],
        }),
    };
    let collection = |kind, items| ResourceCollection {
        cluster_id: detail.cluster_id.clone(),
        session_id: detail.session_id.clone(),
        read_id: 1,
        reference: reference.clone(),
        collection: kind,
        state: CollectionState::Complete,
        items,
    };
    let revisions = [12, 9, 11]
        .into_iter()
        .map(|revision| CollectionItem {
            reference: self::reference(
                ResourceKind::ReplicaSets,
                &format!("checkout-api-revision-{revision}"),
            ),
            created_at: (revision != 9).then(|| CREATED.into()),
            status: "Available".into(),
            columns: columns(&[
                ("revision", &revision.to_string()),
                ("replicas", if revision == 12 { "3" } else { "0" }),
                ("ready", if revision == 12 { "3" } else { "0" }),
            ]),
        })
        .collect();
    let events = (0..4)
        .map(|index| CollectionItem {
            reference: self::reference(
                ResourceKind::Events,
                &format!("checkout-api-event-{index}"),
            ),
            created_at: Some(CREATED.into()),
            status: "Normal".into(),
            columns: columns(&[
                ("reason", "ScalingReplicaSet"),
                (
                    "message",
                    "Scaled up replica set checkout-api-revision-12 to 3",
                ),
                ("source", "deployment-controller"),
                ("count", "1"),
                ("lastSeen", CREATED),
            ]),
        })
        .collect();
    let collections = vec![
        collection(CollectionKind::ReplicaSets, revisions),
        collection(CollectionKind::Events, events),
    ];
    (detail, collections)
}

/// Scheduling fixture uses the same reviewed section/label contract as core facts.
fn daemon_set_fixture(populated: bool) -> (ResourceDetail, Vec<ResourceCollection>) {
    let (mut detail, _) = fixture();
    detail.cluster_id = "offline-daemon-set-probe".into();
    detail.reference = reference(ResourceKind::DaemonSets, "node-agent");
    detail.item.name = detail.reference.name.clone();
    detail.item.uid = detail.reference.uid.clone();
    detail.item.status = if populated { "Progressing" } else { "Ready" }.into();
    let count = if populated { "3" } else { "0" };
    let ready = if populated { "2" } else { "0" };
    detail.item.columns = columns(&[
        ("desired", count),
        ("scheduled", count),
        ("ready", ready),
        ("updated", ready),
        ("available", ready),
        ("nodeSelector", "kubernetes.io/os=linux"),
    ]);
    let section = |title: &str, values: &[(&str, &str)]| DetailSection {
        title: title.into(),
        facts: values
            .iter()
            .map(|(label, value)| DetailFact {
                label: (*label).into(),
                value: (*value).into(),
            })
            .collect(),
    };
    detail.sections = vec![
        section("Selector", &[("Selector · app", "node-agent")]),
        section(
            "Node selector",
            &[("Node selector · kubernetes.io/os", "linux")],
        ),
        section(
            "Container images",
            &[(
                "Container images · 1 · image",
                "registry.example.test/node-agent:v1",
            )],
        ),
        section(
            "Init container images",
            &[(
                "Init container images · 1 · image",
                "registry.example.test/node-init:v1",
            )],
        ),
        section(
            "DaemonSet",
            &[
                ("Desired scheduled", count),
                ("Current scheduled", count),
                ("Ready", ready),
                ("Updated scheduled", ready),
                ("Available", ready),
                ("Unavailable", if populated { "1" } else { "0" }),
                ("Misscheduled", "0"),
                ("Update strategy", "RollingUpdate"),
                ("Minimum ready seconds", "0"),
                ("Observed generation", "7"),
            ],
        ),
        section(
            "DaemonSet rolling update",
            &[
                ("DaemonSet rolling update · maxUnavailable", "1"),
                ("DaemonSet rolling update · maxSurge", "0"),
            ],
        ),
        section(
            "Tolerations",
            &[
                (
                    "Tolerations · 1 · key",
                    "node-role.kubernetes.io/control-plane",
                ),
                ("Tolerations · 1 · operator", "Exists"),
                ("Tolerations · 1 · effect", "NoSchedule"),
            ],
        ),
        section(
            "Affinity",
            &[
                (
                    "Affinity · nodeAffinity · requiredDuringSchedulingIgnoredDuringExecution · nodeSelectorTerms · 1 · matchExpressions · 1 · key",
                    "kubernetes.io/os",
                ),
                (
                    "Affinity · nodeAffinity · requiredDuringSchedulingIgnoredDuringExecution · nodeSelectorTerms · 1 · matchExpressions · 1 · operator",
                    "In",
                ),
                (
                    "Affinity · nodeAffinity · requiredDuringSchedulingIgnoredDuringExecution · nodeSelectorTerms · 1 · matchExpressions · 1 · values · 1",
                    "linux",
                ),
            ],
        ),
    ];
    let metadata = detail.metadata.as_mut().unwrap();
    metadata.labels[0].value = "node-agent".into();
    metadata.labels[1].value = "infrastructure".into();
    metadata
        .annotations
        .retain(|entry| !entry.key.starts_with("deployment."));
    metadata.managed_fields[0].fields =
        "[\"status.desiredNumberScheduled\",\"status.numberReady\"]".into();
    let pods = if populated {
        ["Running", "Running", "Pending"]
            .into_iter()
            .enumerate()
            .map(|(index, status)| CollectionItem {
                reference: reference(ResourceKind::Pods, &format!("node-agent-{index}")),
                created_at: Some(CREATED.into()),
                status: status.into(),
                columns: BTreeMap::new(),
            })
            .collect()
    } else {
        vec![]
    };
    let collections = [
        (CollectionKind::Pods, pods),
        (CollectionKind::Events, vec![]),
    ]
    .into_iter()
    .map(|(collection, items)| ResourceCollection {
        cluster_id: detail.cluster_id.clone(),
        session_id: detail.session_id.clone(),
        read_id: detail.read_id,
        reference: detail.reference.clone(),
        collection,
        state: CollectionState::Complete,
        items,
    })
    .collect();
    (detail, collections)
}

struct Probe {
    owner: Entity<IceonApp>,
    table: Entity<ResourceTable>,
    inspector_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    table_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    renders: usize,
    outcome: Outcome,
    expected: (ResourceDetail, Vec<ResourceCollection>),
    initial_operation: u64,
    _directory: Rc<FixtureDirectory>,
}
impl Probe {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        outcome: Outcome,
        directory: Rc<FixtureDirectory>,
        daemon_set: bool,
        populated: bool,
    ) -> Self {
        let catalog = Arc::new(CatalogEngine::new_with_home(directory.0.clone(), None));
        let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
        let preferences =
            PreferencesEngine::new(directory.0.clone()).expect("isolated preferences");
        let secrets = secrets::Engine::new(directory.0.clone());
        let owner =
            cx.new(|cx| IceonApp::new(catalog, connection, preferences, secrets, window, cx));
        let expected = if daemon_set {
            daemon_set_fixture(populated)
        } else {
            fixture()
        };
        let initial_operation = owner.read(cx).operation_counter;
        owner.update(cx, |app, _| {
            // This flag selects a default console tab; no terminal is spawned.
            app.local_terminal_active = false;
            app.inspector_open = true;
            app.inspector.target = Some(InspectorTarget::Resource(expected.0.reference.clone()));
            app.inspector.detail = Some(expected.0.clone());
            app.inspector.collections = expected.1.clone();
        });
        let table = cx.new(|cx| {
            ResourceTable::new(owner.downgrade(), cx)
                .without_selection()
                .without_pagination()
        });
        table.update(cx, |table, cx| {
            table.sync(
                TableData {
                    page: Some(Rc::new(ResourcePage {
                        cluster_id: expected.0.cluster_id.clone(),
                        session_id: expected.0.session_id.clone(),
                        kind: expected.0.reference.kind,
                        read_id: 1,
                        items: (0..ROWS)
                            .map(|i| {
                                let mut item = expected.0.item.clone();
                                item.name = format!("{}-{i:02}", expected.0.item.name);
                                item.uid = Some(format!("fixture-table-{i}"));
                                item
                            })
                            .collect(),
                        truncated: false,
                        fetched_at: 0,
                        namespace: Some("offline-demo".into()),
                        continue_token: None,
                        metrics: None,
                    })),
                    scope: ResourceScrollScope {
                        cluster_id: expected.0.cluster_id.clone(),
                        kind: expected.0.reference.kind,
                        namespace: None,
                        query: String::new(),
                        status: None,
                        page_token: None,
                    },
                },
                cx,
            )
        });
        Self {
            owner,
            table,
            inspector_bounds: Rc::default(),
            table_bounds: Rc::default(),
            renders: 0,
            outcome,
            expected,
            initial_operation,
            _directory: directory,
        }
    }
    fn check(&self, cx: &App, scrolled: bool) -> Result<(), String> {
        let require = |ok, message: &str| if ok { Ok(()) } else { Err(message.to_owned()) };
        for bounds in [&self.inspector_bounds, &self.table_bounds] {
            let bounds = bounds.get().ok_or("native layout was not measured")?;
            require(
                bounds.size.width > px(100.) && bounds.size.height > px(100.),
                "empty native content bounds",
            )?;
        }
        let app = self.owner.read(cx);
        require(self.renders > 0, "inspector wrapper never rendered")?;
        require(
            app.inspector.detail.as_ref() == Some(&self.expected.0)
                && app.inspector.collections == self.expected.1,
            "render/input mutated read-only fixture data",
        )?;
        require(
            app.cluster_session.is_none() && !app.connection.has_active_runtime(),
            "offline probe acquired a connection",
        )?;
        require(
            app.operation_counter == self.initial_operation
                && app.inspector.read_id == 0
                && app.inspector.action_message.is_none(),
            "unexpected inspector action/request",
        )?;
        require(
            app.device_terminals.is_empty()
                && !app.local_terminal_active
                && app.yaml_dock_tabs.is_empty(),
            "unexpected terminal/editor action",
        )?;
        for action in [WorkloadAction::Scale, WorkloadAction::Restart] {
            require(
                !app.can_workload_action(&self.expected.0.reference, action),
                "disconnected mutation unexpectedly enabled",
            )?;
        }
        let stats = self.table.read(cx).stats();
        require(
            stats.row_renders > 0,
            "real workload table rendered no rows",
        )?;
        if scrolled {
            require(
                self.table.read(cx).scroll_offsets().0.y < px(0.),
                "wheel input did not scroll the real resource table",
            )?;
        }
        Ok(())
    }
}
impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let inspector = self.owner.update(cx, |app, cx| {
            app.render_inspector_drawer(window, cx).into_any_element()
        });
        let inspector_bounds = self.inspector_bounds.clone();
        let table_bounds = self.table_bounds.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .h(px(190.))
                    .flex_shrink_0()
                    .relative()
                    .overflow_hidden()
                    .child(self.table.clone())
                    .child(
                        canvas(
                            move |bounds, _, _| table_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .child(inspector)
                    .child(
                        canvas(
                            move |bounds, _, _| inspector_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    ),
            )
    }
}
fn wheel(window: &mut Window, cx: &mut App, bounds: Bounds<Pixels>) {
    window.dispatch_event(
        PlatformInput::Scroll(ScrollEvent {
            position: bounds.origin + point(bounds.size.width / 2., bounds.size.height * 0.7),
            delta: ScrollDelta::Pixels(point(px(0.), px(-420.))),
            ..Default::default()
        }),
        cx,
    );
}

pub fn run() {
    eprintln!("deployment_probe: starting isolated native audit");
    let preview = std::env::args().any(|arg| arg == "--preview");
    let light = std::env::args().any(|arg| arg == "--light");
    let compact = std::env::args().any(|arg| arg == "--compact");
    let daemon_set = std::env::args().any(|arg| arg == "--daemon-set");
    let populated = std::env::args().any(|arg| arg == "--populated")
        && !std::env::args().any(|arg| arg == "--empty");
    let kind_name = if daemon_set {
        "DaemonSet"
    } else {
        "Deployment"
    };
    let runtime = iceon_core::runtime::handle();
    let _guard = runtime.enter();
    let directory = Rc::new(FixtureDirectory::new());
    let outcome: Outcome = Rc::default();
    let result = outcome.clone();
    gpui_kit::application().with_assets(crate::app_assets::AppAssets).run(move |cx| {
        eprintln!("deployment_probe: native application launch callback");
        gpui_kit::init(cx);
        crate::console_font::init(cx);
        crate::shell_theme::apply(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
        let (handle, view) = crate::shell_theme::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(if compact { 560. } else { 960. }), px(850.)), cx))),
            ..Default::default()
        }, cx, { let result = result.clone(); move |window, cx| {
            window.set_window_title(&format!("Iceon {kind_name} — offline native probe"));
            cx.new(|cx| Probe::new(window, cx, result, directory, daemon_set, populated))
        }}).expect("open isolated workload window");
        eprintln!("deployment_probe: isolated native window constructed");
        let watchdog = result.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(20)).await;
            cx.update(|cx| {
                if watchdog.borrow().is_none() { *watchdog.borrow_mut() = Some(Err("native probe watchdog expired".into())); }
                cx.quit();
            });
        }).detach();
        cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
        cx.spawn(async move |cx| {
            for stage in 0..3 {
                cx.background_executor().timer(Duration::from_millis(450)).await;
                let checked = handle.update(cx, |_, window, cx| {
                    // Explicit draws also cover an occluded native window.
                    window.draw(cx).clear(cx);
                    let checked = view.update(cx, |probe, cx| probe.check(cx, stage > 0));
                    if let Err(error) = &checked {
                        eprintln!("FAIL deployment_probe stage {stage}: {error}");
                        *result.borrow_mut() = Some(Err(error.clone())); cx.quit(); return false;
                    }
                    if stage == 0 {
                        let bounds = view.read(cx).table_bounds.get().unwrap();
                        wheel(window, cx, bounds);
                    } else if stage == 1 {
                        let bounds = view.read(cx).inspector_bounds.get().unwrap();
                        wheel(window, cx, bounds);
                    } else {
                        let probe = view.read(cx);
                        println!("PASS deployment_probe: real native {kind_name} inspector bounds={:?}; real table rows={} scroll={:?}; inspector wheel dispatched; fixture unchanged; disconnected actions disabled; no connection/shell", probe.inspector_bounds.get(), probe.table.read(cx).stats().row_renders, probe.table.read(cx).scroll_offsets().0);
                        *probe.outcome.borrow_mut() = Some(Ok(()));
                        if !preview { cx.quit(); }
                    }
                    true
                });
                if !matches!(checked, Ok(true)) { return; }
            }
            if preview {
                cx.background_executor().timer(Duration::from_secs(12)).await;
                cx.update(|cx| cx.quit());
            }
        }).detach();
        cx.activate(true);
    });
    match outcome.borrow_mut().take() {
        Some(Ok(())) => {}
        Some(Err(error)) => panic!("FAIL deployment_probe: {error}"),
        None => panic!("FAIL deployment_probe: window closed before verification"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn daemon_set_fixture_preserves_zero_counts_reviewed_images_and_scheduling() {
        let (detail, collections) = daemon_set_fixture(false);
        assert_eq!(detail.reference.kind, ResourceKind::DaemonSets);
        for key in ["desired", "scheduled", "ready", "updated", "available"] {
            assert_eq!(detail.item.columns[key], "0", "{key}");
        }
        assert_eq!(
            detail.item.columns["nodeSelector"],
            "kubernetes.io/os=linux"
        );
        assert!(!detail.item.columns.contains_key("replicas"));
        let facts = |title: &str| {
            detail
                .sections
                .iter()
                .find(|s| s.title == title)
                .unwrap()
                .facts
                .clone()
        };
        assert_eq!(
            facts("Node selector")[0].label,
            "Node selector · kubernetes.io/os"
        );
        assert_eq!(facts("Node selector")[0].value, "linux");
        for title in ["Container images", "Init container images"] {
            let images = facts(title);
            assert_eq!(images[0].label, format!("{title} · 1 · image"));
            assert!(images[0].value.starts_with("registry.example.test/"));
        }
        assert!(
            facts("DaemonSet")
                .iter()
                .any(|f| f.label == "Update strategy" && f.value == "RollingUpdate")
        );
        assert_eq!(facts("Tolerations").len(), 3);
        assert_eq!(facts("Affinity").len(), 3);
        assert_eq!(detail.item.resource_version.as_deref(), Some("42"));
        let metadata = detail.metadata.unwrap();
        assert!(!metadata.labels.is_empty() && !metadata.managed_fields.is_empty());
        assert!(
            metadata
                .annotations
                .iter()
                .all(|e| !e.key.contains("revision"))
        );
        assert!(
            detail
                .sections
                .iter()
                .all(|s| !s.title.contains("Deployment") && !s.title.contains("Replicas"))
        );
        assert_eq!(collections.len(), 2);
        assert_eq!(collections[0].collection, CollectionKind::Pods);
        assert_eq!(collections[1].collection, CollectionKind::Events);
        for collection in collections {
            assert_eq!(collection.reference, detail.reference);
            assert_eq!(collection.state, CollectionState::Complete);
            assert!(collection.items.is_empty());
            assert_ne!(collection.collection, CollectionKind::ReplicaSets);
        }
    }

    #[::core::prelude::v1::test]
    fn populated_daemon_set_counts_agree_with_owned_pod_statuses_without_revisions() {
        let (detail, collections) = daemon_set_fixture(true);
        let pods = &collections[0];
        assert_eq!(detail.item.columns["desired"], pods.items.len().to_string());
        assert_eq!(
            detail.item.columns["scheduled"],
            pods.items.len().to_string()
        );
        let running = pods.items.iter().filter(|p| p.status == "Running").count();
        for key in ["ready", "updated", "available"] {
            assert_eq!(detail.item.columns[key], running.to_string());
        }
        assert_eq!(
            pods.items.iter().filter(|p| p.status == "Pending").count(),
            1
        );
        assert!(
            pods.items
                .iter()
                .all(|p| p.reference.kind == ResourceKind::Pods)
        );
        assert!(
            collections
                .iter()
                .all(|c| c.collection != CollectionKind::ReplicaSets)
        );
        assert!(collections[1].items.is_empty());
    }

    #[::core::prelude::v1::test]
    fn offline_fixture_has_rollout_metadata_revisions_and_events() {
        let (detail, collections) = fixture();
        assert_eq!(detail.item.columns["conditions"], "Available,Progressing");
        assert_eq!(detail.item.columns["ready"], "3");
        assert_eq!(detail.sections.len(), 4);
        let metadata = detail.metadata.unwrap();
        assert!(!metadata.labels.is_empty() && !metadata.annotations.is_empty());
        assert_eq!(metadata.managed_fields[0].operation, "Update");
        assert_eq!(
            collections[0]
                .items
                .iter()
                .map(|item| item.columns["revision"].as_str())
                .collect::<Vec<_>>(),
            ["12", "9", "11"]
        );
        assert!(collections[0].items[1].created_at.is_none());
        assert_eq!(collections[1].collection, CollectionKind::Events);
        for event in &collections[1].items {
            for key in ["message", "source", "count", "lastSeen"] {
                assert!(!event.columns[key].is_empty());
            }
        }
    }
}

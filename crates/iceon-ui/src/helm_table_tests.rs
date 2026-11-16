use super::*;

fn release(name: &str, revision: Option<u32>) -> HelmItem {
    HelmItem {
        id: name.into(),
        name: name.into(),
        namespace: Some("monitor".into()),
        version: "prometheus-node-exporter-4.46.0".into(),
        app_version: "1.8.2".into(),
        description: None,
        status: Some("deployed".into()),
        revision,
        updated: Some("2026-11-16 07:00:00.123456 +0000 UTC".into()),
    }
}

#[::core::prelude::v1::test]
fn release_columns_match_inventory_and_charts_have_no_fabricated_status() {
    assert_eq!(
        columns(HelmView::Releases)
            .iter()
            .map(|c| c.label())
            .collect::<Vec<_>>(),
        [
            "Name",
            "Namespace",
            "Chart",
            "Revision",
            "Version",
            "App Version",
            "Status",
            "Updated",
            "Actions"
        ]
    );
    assert_eq!(
        columns(HelmView::Charts)
            .iter()
            .map(|c| c.label())
            .collect::<Vec<_>>(),
        ["Name", "Version", "App Version", "Description", "Actions"]
    );
}

#[::core::prelude::v1::test]
fn chart_labels_preserve_unknown_versions_instead_of_guessing() {
    assert_eq!(
        chart_parts("prometheus-node-exporter-4.46.0"),
        ("prometheus-node-exporter", "4.46.0")
    );
    for label in [
        "",
        "custom",
        "widget-1.2",
        "widget-01.2.3",
        "widget-1.2.3-beta.1",
    ] {
        assert_eq!(chart_parts(label), (label, "—"));
    }
}

#[::core::prelude::v1::test]
fn search_includes_release_fields_and_revision_sort_is_numeric_and_stable() {
    let items = vec![
        release("a", Some(10)),
        release("b", Some(2)),
        release("c", Some(2)),
    ];
    for query in ["monitor", "node-exporter", "deployed", "1.8.2", "4.46.0"] {
        assert_eq!(
            sorted_rows(&items, HelmView::Releases, query, None),
            [0, 1, 2]
        );
    }
    assert!(sorted_rows(&items, HelmView::Releases, "absent", None).is_empty());
    assert_eq!(
        sorted_rows(
            &items,
            HelmView::Releases,
            "",
            Some((Column::Revision, false))
        ),
        [1, 2, 0]
    );
    assert_eq!(
        sorted_rows(
            &items,
            HelmView::Releases,
            "",
            Some((Column::Revision, true))
        ),
        [0, 1, 2]
    );
    assert_eq!(sorted_rows(&items, HelmView::Releases, "", None), [0, 1, 2]);
}

#[::core::prelude::v1::test]
fn missing_observations_remain_missing() {
    let mut item = release("empty", None);
    item.namespace = None;
    item.status = None;
    item.app_version.clear();
    for column in [
        Column::Namespace,
        Column::Status,
        Column::Revision,
        Column::AppVersion,
    ] {
        assert_eq!(value(&item, HelmView::Releases, column), "—");
    }
}

#[::core::prelude::v1::test]
fn updated_times_preserve_offsets_and_unknown_text() {
    let go = normalized_updated(Some("2026-11-16 14:00:00.123456 +0700 WIB")).unwrap();
    assert_eq!(
        go.parse::<jiff::Timestamp>().unwrap(),
        "2026-11-16T07:00:00.123456Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
    );
    assert_eq!(normalized_updated(None), None);
    assert_eq!(normalized_updated(Some("")), None);
    assert_eq!(
        normalized_updated(Some("not reported")),
        Some("not reported".into())
    );
}

//! Offline projection and request-identity probes; no process or cluster access.
use super::*;

fn request() -> HelmDetailRequest {
    HelmDetailRequest {
        cluster_id: "offline-cluster".into(),
        session_id: "lease-a".into(),
        read_id: 4,
        view: HelmView::Releases,
        name: "fixture".into(),
        namespace: Some("sandbox".into()),
        version: None,
    }
}
fn detail() -> HelmDetail {
    let request = request();
    HelmDetail {
        cluster_id: request.cluster_id,
        session_id: request.session_id,
        read_id: request.read_id,
        view: request.view,
        name: request.name,
        namespace: request.namespace,
        version: None,
        facts: vec![],
        history: vec![],
        truncated: false,
    }
}
#[::core::prelude::v1::test]
fn helm_inspector_metadata_uses_highest_returned_revision_without_inventing_missing_fields() {
    let mut detail = detail();
    assert!(latest_returned(&detail).is_none());
    assert_eq!(chart_parts(""), ("—", "—"));
    assert_eq!(available(None), "—");
    assert_eq!(available(Some("  ")), "—");
    for revision in [7, 2, 4] {
        detail.history.push(HelmHistory {
            revision,
            updated: String::new(),
            status: "deployed".into(),
            chart: "test-chart-1.2.3".into(),
            app_version: String::new(),
        });
    }
    let latest = latest_returned(&detail).unwrap();
    assert_eq!(latest.revision, 7);
    assert_eq!(chart_parts(&latest.chart), ("test-chart", "1.2.3"));
    assert_eq!(available(Some(&latest.app_version)), "—");
    assert_eq!(history_label(&detail), "Revision history (3)");
    detail.truncated = true;
    assert_eq!(
        history_label(&detail),
        "Revision history (3 shown · partial)"
    );
}
#[::core::prelude::v1::test]
fn helm_inspector_unfamiliar_chart_labels_are_not_guessed() {
    for label in [
        "chart",
        "chart-1.2",
        "chart-1.2.3-beta.1",
        "chart-01.2.3",
        "chart-1.2.3+build",
    ] {
        assert_eq!(chart_parts(label), (label, "—"));
    }
}
#[::core::prelude::v1::test]
fn helm_inspector_install_never_substitutes_a_missing_version() {
    for version in [None, Some(""), Some(" "), Some("--version"), Some("1.0\n")] {
        assert_eq!(install_version(version), None);
    }
    assert_eq!(
        install_version(Some("1.2.3-rc.1+build")),
        Some("1.2.3-rc.1+build")
    );
}
#[::core::prelude::v1::test]
fn helm_inspector_dates_keep_numeric_timezone_without_guessing() {
    assert_eq!(
        normalized_updated("2026-01-02 12:30:00.123 +0700 WIB"),
        Some("2026-01-02T05:30:00.123Z".into())
    );
    assert_eq!(
        normalized_updated("2026-01-02T05:30:00Z"),
        Some("2026-01-02T05:30:00Z".into())
    );
    assert_eq!(normalized_updated("yesterday"), None);
    assert_eq!(normalized_updated(""), None);
}
#[::core::prelude::v1::test]
fn helm_inspector_identity_rejects_replacement_session_target_and_refresh() {
    let original = request();
    assert!(same_request(&original, &request()));
    for field in 0..7 {
        let mut replaced = request();
        match field {
            0 => replaced.cluster_id = "other-cluster".into(),
            1 => replaced.session_id = "lease-b".into(),
            2 => replaced.read_id += 1,
            3 => replaced.name = "other-release".into(),
            4 => replaced.namespace = Some("other-namespace".into()),
            5 => replaced.version = Some("1.0.0".into()),
            _ => replaced.view = HelmView::Charts,
        }
        assert!(!same_request(&original, &replaced));
    }
    let state = InspectorState::default();
    assert!(!accepts_read(&state, &original));
}

#[::core::prelude::v1::test]
fn helm_inspector_closed_or_replaced_drawer_rejects_pending_completion() {
    let request = request();
    let target = InspectorTarget::Helm {
        view: request.view,
        name: request.name.clone(),
        namespace: request.namespace.clone(),
        version: request.version.clone(),
    };
    assert!(accepts_request(4, Some(&target), Some(&request), &request));
    assert!(!accepts_request(5, Some(&target), Some(&request), &request));
    assert!(!accepts_request(4, None, Some(&request), &request));
    assert!(!accepts_request(4, Some(&target), None, &request));
    let replacement = InspectorTarget::Helm {
        view: request.view,
        name: "replacement".into(),
        namespace: request.namespace.clone(),
        version: None,
    };
    assert!(!accepts_request(
        4,
        Some(&replacement),
        Some(&request),
        &request
    ));
}

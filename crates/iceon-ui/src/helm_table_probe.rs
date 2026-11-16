//! Offline, actual-native Helm table fixture. No connection or application shell is constructed.
use crate::helm_table::HelmTable;
use gpui_kit::prelude::*;
use gpui_kit::private::anyhow;
use gpui_kit::*;
use iceon_core::connection::helm::{HelmItem, HelmPage, HelmView};
use std::rc::Rc;

pub fn run() -> anyhow::Result<()> {
    use gpui_kit::component::ThemeMode;
    use std::{cell::RefCell, time::Duration};
    type Outcome = Rc<RefCell<Option<Result<(), String>>>>;
    let outcome: Outcome = Rc::default();
    let result = outcome.clone();
    gpui_kit::application()
        .with_assets(crate::app_assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::console_font::init(cx);
            crate::shell_theme::apply(ThemeMode::Dark, None, cx);
            let opened = crate::shell_theme::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None, size(px(1120.), px(640.)), cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Iceon Helm — offline native probe");
                    cx.new(|cx| {
                        let mut table = HelmTable::new(window, cx);
                        table.sync(Some(fixture(HelmView::Releases)), HelmView::Releases, false, None, "UTC".into(), cx);
                        table
                    })
                },
            );
            let (handle, table) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    *result.borrow_mut() = Some(Err(error.to_string()));
                    cx.quit();
                    return;
                }
            };
            let events = Rc::new(std::cell::Cell::new(0usize));
            let observed = events.clone();
            cx.subscribe(&table, move |_, _: &crate::helm_table::HelmTableEvent, _| {
                observed.set(observed.get() + 1);
            }).detach();
            let watchdog = result.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(15)).await;
                cx.update(|cx| {
                    if watchdog.borrow().is_none() {
                        *watchdog.borrow_mut() = Some(Err("native watchdog expired".into()));
                    }
                    cx.quit();
                });
            }).detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() { cx.quit(); }
            }).detach();
            cx.spawn(async move |cx| {
                // Each mutation is followed by a later native draw: search is
                // driven through InputState so its real observer must fire.
                let mut previous_renders = 0;
                let mut previous_rows = 0;
                for stage in 0..17 {
                    cx.background_executor().timer(Duration::from_millis(220)).await;
                    let checked = handle.update(cx, |_, window, cx| -> anyhow::Result<()> {
                        window.draw(cx).clear(cx);
                        table.update(cx, |this, cx| -> anyhow::Result<()> {
                            anyhow::ensure!(events.get() == 0, "search/sort emitted an action event");
                            let state = this.probe_snapshot();
                            let count = this.inventory_count().map(|count| count.label());
                            anyhow::ensure!(count.is_some() == state.page.is_some(), "missing snapshots must not claim zero items");
                            if let Some(count) = count {
                                anyhow::ensure!(count.starts_with(&state.rows.len().to_string()), "header count did not react to cached search");
                            }
                            anyhow::ensure!(state.view_renders > previous_renders, "stage {stage}: view did not render");
                            previous_renders = state.view_renders;
                            let expected = match stage { 4 => 40, 5 => 10, 8 | 12..=16 => 0, 11 => 1, _ => 80 };
                            anyhow::ensure!(state.rows.len() == expected && state.sizes == expected, "stage {stage}: expected {expected} rows and sizes, got {} / {}", state.rows.len(), state.sizes);
                            if expected > 0 {
                                anyhow::ensure!(state.row_renders > previous_rows, "stage {stage}: actual virtual row callback did not render");
                            }
                            previous_rows = state.row_renders;
                            // All stages are local projections. Search/sort must never
                            // alter the supplied inventory or emit mutation events.
                            if stage <= 9 {
                                anyhow::ensure!(state.page == Some(fixture(HelmView::Releases)), "release inventory mutated");
                            } else if stage <= 12 {
                                anyhow::ensure!(state.page == Some(fixture(HelmView::Charts)), "chart inventory mutated");
                            }
                            match stage {
                                0 => this.probe_sort_revision(cx),
                                1 => {
                                    anyhow::ensure!(state.revision_sort == Some(false) && state.rows[0] == 1 && state.rows[79] == 0, "numeric ascending revision sort failed (2 before 10)");
                                    this.probe_sort_revision(cx);
                                }
                                2 => {
                                    anyhow::ensure!(state.revision_sort == Some(true) && state.rows[0] == 0 && state.rows[79] == 1, "descending revision sort failed");
                                    this.probe_sort_revision(cx);
                                }
                                3 => {
                                    anyhow::ensure!(state.revision_sort.is_none() && state.rows == (0..80).collect::<Vec<_>>(), "third sort did not restore source order");
                                    this.probe_query("team-even", window, cx);
                                }
                                4 => {
                                    anyhow::ensure!(state.rows.iter().all(|i| i % 2 == 0), "namespace filter failed");
                                    this.probe_query("pending-upgrade", window, cx);
                                }
                                5 => {
                                    anyhow::ensure!(state.rows.iter().all(|i| i % 8 == 3), "status filter failed");
                                    this.probe_query("node-exporter", window, cx);
                                }
                                6 => this.probe_query("1.8.2", window, cx),
                                7 => this.probe_query("no-such-offline-release", window, cx),
                                8 => {
                                    this.probe_query("", window, cx);
                                    this.sync(Some(fixture(HelmView::Releases)), HelmView::Releases, true, None, "UTC".into(), cx);
                                }
                                9 => {
                                    anyhow::ensure!(state.loading && state.page.as_ref().unwrap().truncated, "partial cached loading state missing");
                                    this.sync(Some(fixture(HelmView::Charts)), HelmView::Charts, false, None, "UTC".into(), cx);
                                }
                                10 => this.probe_query("unique chart description", window, cx),
                                11 => {
                                    anyhow::ensure!(state.rows == [37], "chart description search failed");
                                    this.probe_query("no-such-offline-chart", window, cx);
                                }
                                12 => this.sync(None, HelmView::Charts, true, None, "UTC".into(), cx),
                                13 => {
                                    anyhow::ensure!(state.loading && state.page.is_none(), "empty loading state missing");
                                    this.sync(None, HelmView::Charts, false, Some("Offline fixture error".into()), "UTC".into(), cx);
                                }
                                14 => {
                                    anyhow::ensure!(!state.loading && state.error.as_deref() == Some("Offline fixture error"), "error state missing");
                                    let mut empty = fixture(HelmView::Charts);
                                    empty.items.clear();
                                    empty.truncated = false;
                                    this.sync(Some(empty), HelmView::Charts, false, None, "UTC".into(), cx);
                                }
                                15 => this.probe_query("", window, cx),
                                16 => anyhow::ensure!(!state.loading && state.error.is_none() && state.page.as_ref().unwrap().items.is_empty(), "true empty inventory state missing"),
                                _ => unreachable!(),
                            }
                            Ok(())
                        })
                    });
                    let error = match checked {
                        Ok(Ok(())) => None,
                        Ok(Err(error)) => Some(error.to_string()),
                        Err(error) => Some(error.to_string()),
                    };
                    if let Some(error) = error {
                        cx.update(|cx| {
                            *result.borrow_mut() = Some(Err(format!("stage {stage}: {error}")));
                            cx.quit();
                        });
                        return;
                    }
                }
                cx.update(|cx| {
                    println!("HELM_TABLE_PROBE_OK: actual native HelmTable and virtual rows rendered; 80 synthetic releases and charts; numeric sort cycle, InputState search, partial cached loading, empty and error verified; no engines, credentials or Helm commands");
                    *result.borrow_mut() = Some(Ok(()));
                    cx.quit();
                });
            }).detach();
            cx.activate(true);
        });
    let result = outcome.borrow_mut().take();
    match result {
        Some(Ok(())) => Ok(()),
        Some(Err(error)) => anyhow::bail!("Helm table probe: {error}"),
        None => anyhow::bail!("Helm table probe closed before verification"),
    }
}

fn fixture(view: HelmView) -> HelmPage {
    HelmPage {
        cluster_id: "offline-fixture".into(),
        session_id: "offline-session".into(),
        read_id: 1,
        namespace: None,
        view,
        truncated: true,
        items: (0..80)
            .map(|i| HelmItem {
                id: format!("offline-{i}"),
                name: format!("fixture-{:03}", 79 - i),
                namespace: (view == HelmView::Releases)
                    .then(|| if i % 2 == 0 { "team-even" } else { "team-odd" }.into()),
                version: if view == HelmView::Releases {
                    "prometheus-node-exporter-4.46.0"
                } else {
                    "4.46.0"
                }
                .into(),
                app_version: "1.8.2".into(),
                revision: (view == HelmView::Releases).then_some(if i == 0 {
                    100
                } else if i == 1 {
                    2
                } else {
                    i as u32 + 8
                }),
                status: (view == HelmView::Releases).then(|| {
                    [
                        "deployed",
                        "failed",
                        "pending-install",
                        "pending-upgrade",
                        "pending-rollback",
                        "uninstalling",
                        "superseded",
                        "unknown",
                    ][i % 8]
                        .into()
                }),
                updated: (view == HelmView::Releases)
                    .then(|| "2026-11-16 14:00:00.123456 +0700 WIB".into()),
                description: (view == HelmView::Charts).then(|| {
                    if i == 37 {
                        "unique chart description"
                    } else {
                        "Synthetic offline chart"
                    }
                    .into()
                }),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn fixtures_are_synthetic_and_cover_release_and_chart_projections() {
        let releases = fixture(HelmView::Releases);
        assert_eq!(releases.items.len(), 80);
        assert!(releases.truncated);
        assert_eq!(releases.items[1].revision, Some(2));
        assert_eq!(
            releases
                .items
                .iter()
                .filter(|i| i.status.as_deref() == Some("pending-upgrade"))
                .count(),
            10
        );
        assert!(
            releases
                .items
                .iter()
                .all(|i| i.updated.as_deref().unwrap().contains("+0700 WIB"))
        );
        let charts = fixture(HelmView::Charts);
        assert_eq!(charts.items.len(), 80);
        assert!(charts.items.iter().all(|i| i.revision.is_none()
            && i.status.is_none()
            && i.updated.is_none()
            && i.namespace.is_none()));
    }
}

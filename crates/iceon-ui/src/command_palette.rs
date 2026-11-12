//! Global navigation through GPUI Kit's keyboard-accessible Command component.
use crate::{
    IceonApp,
    navigation::{Route, destination_label},
};
use gpui_kit::{
    component::{
        command::{Command, CommandItem},
        h_flex,
    },
    prelude::*,
    *,
};

fn destinations(
    clusters: &[iceon_core::catalog::ClusterSummary],
    current: &Route,
) -> Vec<(String, Vec<String>, Route)> {
    let mut entries = vec![
        (
            "Home · Clusters".into(),
            vec!["catalog".into()],
            Route::Catalog,
        ),
        ("Secrets".into(), vec!["vault".into()], Route::Secrets),
    ];
    entries.extend(clusters.iter().map(|cluster| {
        (
            format!("Cluster · {}", cluster.name),
            vec![cluster.context_name.clone()],
            Route::Cluster {
                cluster_id: cluster.id.clone(),
                section: "overview".into(),
            },
        )
    }));
    let cluster_id = match current {
        Route::Cluster { cluster_id, .. } => Some(cluster_id.clone()),
        _ => clusters.first().map(|cluster| cluster.id.clone()),
    };
    if let Some(cluster_id) = cluster_id {
        entries.extend(iceon_core::catalog::SECTIONS.iter().map(|section| {
            (
                format!("Section · {}", destination_label(section)),
                vec![(*section).into()],
                Route::Cluster {
                    cluster_id: cluster_id.clone(),
                    section: (*section).into(),
                },
            )
        }));
    }
    entries
}
impl IceonApp {
    pub(crate) fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_modal_open {
            self.close_command_palette(window, cx);
            return;
        }
        self.search_modal_open = true;
        self.command_state.update(cx, |state, cx| {
            state.set_query("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
    pub(crate) fn close_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_modal_open = false;
        self.shell_focus.focus(window, cx);
        cx.notify();
    }
    pub(crate) fn render_search_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let destinations = destinations(&self.catalog_snapshot.clusters, self.history.current());
        let items = destinations
            .iter()
            .map(|(label, keywords, _)| {
                CommandItem::new()
                    .label(label.clone())
                    .keywords(keywords.clone())
            })
            .collect::<Vec<_>>();
        let owner = cx.entity().downgrade();
        let cancel_owner = owner.clone();
        Command::new(&self.command_state)
            .w(px(520.0))
            .max_w(relative(0.9))
            .max_h(px(320.0))
            .placeholder("Jump to cluster or section…")
            .items(items)
            .on_confirm(move |index, window, cx| {
                if let Some((_, _, route)) = destinations.get(index.row) {
                    let _ = owner.update(cx, |app, cx| {
                        app.close_command_palette(window, cx);
                        app.navigate(route.clone(), true, window, cx);
                    });
                }
            })
            .on_cancel(move |window, cx| {
                let _ = cancel_owner.update(cx, |app, cx| app.close_command_palette(window, cx));
            })
            .footer(|_, _, _| {
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .text_xs()
                    .child("↑↓ Navigate")
                    .child("Enter Open")
                    .child("Esc Clear / Close")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{Route, destinations};
    #[test]
    fn no_cluster_has_only_global_routes() {
        let entries = destinations(&[], &Route::Catalog);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].2, Route::Catalog);
        assert_eq!(entries[1].2, Route::Secrets);
    }
    #[test]
    fn sections_target_the_current_cluster_and_are_not_truncated() {
        let entries = destinations(
            &[],
            &Route::Cluster {
                cluster_id: "active".into(),
                section: "pods".into(),
            },
        );
        assert_eq!(entries.len(), 2 + iceon_core::catalog::SECTIONS.len());
        for (_, _, route) in entries.iter().skip(2) {
            assert!(matches!(route, Route::Cluster { cluster_id, .. } if cluster_id == "active"));
        }
    }
}

//! Compact pod container indicators; hover reveals runtime metadata only.
use gpui_kit::{
    component::{ActiveTheme as _, h_flex, tooltip::Tooltip},
    prelude::*,
    *,
};
use iceon_core::connection::{ResourceItem, container_summary::ContainerSummary};

fn container_details(container: &ContainerSummary) -> Vec<String> {
    let mut details = vec![
        format!(
            "{} · {}{} · {}",
            container.name,
            container.state,
            if container.init { " · init" } else { "" },
            if container.ready {
                "ready"
            } else {
                "not ready"
            }
        ),
        format!("Restarts: {}", container.restarts),
    ];
    if let Some(reason) = &container.reason {
        details.push(format!("Reason: {reason}"));
    }
    if let Some(code) = container.exit_code {
        details.push(format!("Exit Code: {code}"));
    }
    if let Some(start) = &container.started_at {
        details.push(format!("Started At: {start}"));
    }
    if let Some(end) = &container.finished_at {
        details.push(format!("Finished At: {end}"));
    }
    if let Some(id) = &container.container_id {
        details.push(format!("Container ID: {id}"));
    }
    details
}

pub(crate) fn containers(item: &ResourceItem, row: usize, cx: &App) -> AnyElement {
    let containers = item.containers.clone();
    if containers.is_empty() {
        return div().text_xs().child("—").into_any_element();
    }
    let extra = containers.len().saturating_sub(8);
    h_flex()
        .gap_1()
        .items_center()
        .children(
            containers
                .into_iter()
                .take(8)
                .enumerate()
                .map(|(index, container)| {
                    let color = if container.state == "terminated"
                        && container.exit_code.is_some_and(|code| code != 0)
                    {
                        cx.theme().danger
                    } else if container.ready && container.state == "running" {
                        cx.theme().success
                    } else if container.state == "waiting" || container.state == "running" {
                        cx.theme().warning
                    } else {
                        cx.theme().muted_foreground
                    };
                    let filled = !container.init && container.state != "unknown";
                    let details = container_details(&container);
                    div()
                        .id((SharedString::from(format!("pod-container-{row}")), index))
                        .size(px(14.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .when(container.restarts > 0, |el| {
                            el.border_1().border_color(cx.theme().warning)
                        })
                        .child(
                            div()
                                .size(px(8.0))
                                .rounded(px(2.0))
                                .border_1()
                                .border_color(color)
                                .when(filled, |el| el.bg(color)),
                        )
                        .tooltip(move |window, cx| {
                            let details = details.clone();
                            Tooltip::new(details.join("\n")).build(window, cx)
                        })
                }),
        )
        .when(extra > 0, |el| {
            el.child(div().text_xs().child(format!("+{extra}")))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::container_details;
    use iceon_core::connection::container_summary::ContainerSummary;
    #[test]
    fn hover_includes_lifecycle_and_restart_facts() {
        let details = container_details(&ContainerSummary {
            name: "setup".into(),
            init: true,
            state: "terminated".into(),
            ready: false,
            restarts: 2,
            reason: Some("Completed".into()),
            exit_code: Some(0),
            started_at: Some("start".into()),
            finished_at: Some("end".into()),
            container_id: Some("containerd://id".into()),
        });
        let text = details.join("\n");
        for expected in [
            "setup",
            "init",
            "Restarts: 2",
            "Completed",
            "Exit Code: 0",
            "Started At: start",
            "Finished At: end",
            "containerd://id",
        ] {
            assert!(text.contains(expected), "missing {expected}");
        }
    }
}

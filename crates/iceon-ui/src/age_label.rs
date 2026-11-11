//! Live Age labels with one application clock and isolated cached render roots.
//!
//! Use `age_label(stable_id, created_at, timezone)` as a child inside a sized
//! table cell or inspector row. IDs must be unique among sibling labels. No
//! table/shell observer subscribes to the clock. Mount state owns the leaf view;
//! GPUI drops it when that element is no longer rendered, removing its clock
//! subscription. The last unmount cancels the shared timer.

use std::{collections::HashMap, time::Duration};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{navigation::format_age_at, preference_runtime};

#[derive(Default)]
struct AgeClock {
    labels: HashMap<EntityId, WeakEntity<AgeView>>,
    task: Option<Task<()>>,
}
impl Global for AgeClock {}

impl AgeClock {
    fn register(label: Entity<AgeView>, cx: &mut Context<AgeView>) {
        let id = label.entity_id();
        cx.default_global::<Self>()
            .labels
            .insert(id, label.downgrade());
        cx.on_release(move |_, cx| {
            let clock = cx.default_global::<AgeClock>();
            clock.labels.remove(&id);
            if clock.labels.is_empty() {
                clock.task = None;
            }
        })
        .detach();
        if cx.global::<Self>().task.is_some() {
            return;
        }
        let task = cx.spawn(async |_, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                cx.update(|cx| {
                    let labels: Vec<_> = cx.global::<AgeClock>().labels.values().cloned().collect();
                    let now = jiff::Timestamp::now();
                    for label in labels {
                        let _ = label.update(cx, |label, cx| {
                            let text = format_age_at(label.created_at.as_deref(), now);
                            if label.text != text {
                                label.text = text;
                                cx.notify();
                            }
                        });
                    }
                });
            }
        });
        cx.default_global::<Self>().task = Some(task);
    }
}

struct AgeView {
    created_at: Option<String>,
    timezone: String,
    text: String,
}

impl Render for AgeView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let created = self.created_at.clone();
        let timezone = self.timezone.clone();
        div()
            .id("live-age")
            .w_full()
            .child(self.text.clone())
            .when_some(created, |el, created| {
                el.tooltip(move |window, cx| {
                    Tooltip::new(preference_runtime::timestamp(&created, &timezone))
                        .build(window, cx)
                })
            })
    }
}

// Only this non-notifying wrapper uses keyed state. Using AgeView directly as
// keyed state would install GPUI's automatic parent observer and invalidate the
// whole resource table every second.
struct AgeMount(Entity<AgeView>);

#[derive(IntoElement)]
pub struct AgeLabel {
    id: ElementId,
    created_at: Option<String>,
    timezone: String,
}

/// A live label occupying its parent's width and one 20px line. Text styling is
/// inherited, allowing tables and inspector rows to choose their own typography.
pub fn age_label(id: impl Into<ElementId>, created_at: Option<&str>, timezone: &str) -> AgeLabel {
    AgeLabel {
        id: id.into(),
        created_at: created_at.map(str::to_owned),
        timezone: timezone.to_owned(),
    }
}

impl RenderOnce for AgeLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let created_at = self.created_at;
        let timezone = self.timezone;
        let mount = window.use_keyed_state(self.id, cx, |_, cx| {
            AgeMount(cx.new(|cx| {
                let label = AgeView {
                    text: format_age_at(created_at.as_deref(), jiff::Timestamp::now()),
                    created_at: created_at.clone(),
                    timezone: timezone.clone(),
                };
                AgeClock::register(cx.entity(), cx);
                label
            }))
        });
        let view = mount.read(cx).0.clone();
        view.update(cx, |label, cx| {
            if label.created_at != created_at || label.timezone != timezone {
                label.text = format_age_at(created_at.as_deref(), jiff::Timestamp::now());
                label.created_at = created_at;
                label.timezone = timezone;
                cx.notify();
            }
        });
        view.cached(StyleRefinement::default().w_full().h(px(20.)))
    }
}

//! Offline native geometry fixture: production overview surfaces and capacity grid.
//! No application owner, preferences, credentials, shell, or cluster is started.
use crate::overview_view::{
    capacity_card, capacity_pair, card_surface,
    tests::{node_page, usage},
    workload_grid,
};
use gpui_kit::{
    component::{ActiveTheme as _, Theme, theme::ThemeMode, v_flex},
    prelude::*,
    *,
};
use iceon_core::connection::MetricState;
use std::{cell::Cell, rc::Rc, time::Duration};

struct Preview {
    frames: Rc<Cell<usize>>,
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames.set(self.frames.get() + 1);
        let page = node_page(
            Some(MetricState::Available),
            vec![usage(Some(3.), Some(8.))],
        );
        let missing = node_page(Some(MetricState::Partial), vec![usage(None, Some(8.))]);
        let over = node_page(
            Some(MetricState::Available),
            vec![usage(Some(12.), Some(8.))],
        );
        let columns = [
            vec![
                card_surface(cx)
                    .p_4()
                    .child("Jobs · offline fixture")
                    .into_any_element(),
            ],
            vec![
                card_surface(cx)
                    .p_4()
                    .child("DaemonSets · offline fixture")
                    .into_any_element(),
                capacity_pair(
                    capacity_card(Some(&page), false, false, cx),
                    capacity_card(Some(&page), true, false, cx),
                ),
            ],
        ];
        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .bg(cx.theme().background)
            .child(workload_grid(columns))
            .child(capacity_pair(
                capacity_card(Some(&missing), false, false, cx),
                capacity_card(Some(&over), true, false, cx),
            ))
    }
}

pub fn run() {
    let frames = Rc::new(Cell::new(0));
    let compact = std::env::args().any(|arg| arg == "--compact");
    let light = std::env::args().any(|arg| arg == "--light");
    gpui_kit::application().with_assets(crate::app_assets::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(if light { ThemeMode::Light } else { ThemeMode::Dark }, None, cx);
        let rendered = frames.clone();
        gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None,
                size(px(if compact { 400. } else { 1000. }), px(780.)), cx))),
            ..Default::default()
        }, cx, move |window, cx| {
            window.set_window_title("Iceon overview — offline fixture");
            cx.new(|_| Preview { frames: rendered })
        }).expect("offline overview window");
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(4)).await;
            cx.update(|cx| {
                assert!(frames.get() > 0, "overview fixture must actually render");
                println!("PASS overview native fixture: rendered surfaces, capacity pair, partial and overcapacity states");
                cx.quit();
            });
        }).detach();
        cx.activate(true);
    });
}

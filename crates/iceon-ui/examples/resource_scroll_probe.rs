//! Native virtual-list regression probe; no cluster access or test-support feature.
//! Run: cargo run -p iceon-ui --example resource_scroll_probe --locked

#[path = "../src/resource_scroll.rs"]
mod resource_scroll;

use gpui_kit::component::{v_flex, v_virtual_list};
use gpui_kit::{
    AppContext as _, Context, InputEvent as _, IntoElement, ParentElement as _, Render,
    ScrollDelta, ScrollEvent, Styled as _, Window, WindowBounds, WindowOptions, div, point, px,
    size,
};
use iceon_core::connection::ResourceKind;
use resource_scroll::{ResourceScrollScope, ResourceScrollState};
use std::{cell::RefCell, ops::Range, rc::Rc, time::Duration};

struct Probe {
    scroll: ResourceScrollState,
    ranges: Rc<RefCell<Vec<Range<usize>>>>,
}

fn scope() -> ResourceScrollScope {
    ResourceScrollScope {
        cluster_id: "scroll-probe".into(),
        kind: ResourceKind::Pods,
        namespace: None,
        query: String::new(),
        status: None,
        page_token: None,
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.scroll.for_scope(scope());
        let ranges = self.ranges.clone();
        v_flex()
            .size_full()
            .overflow_hidden()
            .child(
                div()
                    .h(px(42.))
                    .flex_shrink_0()
                    .child("Resource table scroll probe"),
            )
            .child(
                v_virtual_list(
                    cx.entity(),
                    "resource-scroll-probe",
                    Rc::new(vec![size(px(600.), px(42.)); 100]),
                    move |_, range, _, _| {
                        ranges.borrow_mut().push(range.clone());
                        range
                            .map(|i| div().w_full().h(px(42.)).child(format!("Pod {i}")))
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&handle)
                .flex_1()
                .min_h_0()
                .w_full(),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let ranges = Rc::new(RefCell::new(Vec::new()));
        let (handle, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds::new(
                    point(px(100.), px(100.)), size(px(600.), px(300.)),
                ))),
                focus: false,
                ..Default::default()
            },
            cx,
            {
                let ranges = ranges.clone();
                move |_, cx| cx.new(|_| Probe { scroll: ResourceScrollState::default(), ranges })
            },
        ).expect("open isolated scroll probe");
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(150)).await;
            handle.update(cx, |_, window, cx| {
                window.draw(cx).clear(cx);
                let initial = ranges.borrow().last().cloned().expect("initial visible rows");
                assert_eq!(initial.start, 0);
                assert!(initial.end < 100, "rendering must stay virtualized");
                window.dispatch_event(ScrollEvent {
                    position: point(px(100.), px(100.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-420.))),
                    ..Default::default()
                }.to_platform_input(), cx);
                window.draw(cx).clear(cx);
                let scrolled = ranges.borrow().last().cloned().unwrap();
                assert!(scrolled.start > initial.start, "wheel must move visible rows: {initial:?} -> {scrolled:?}");
                view.update(cx, |_, cx| cx.notify());
                window.draw(cx).clear(cx);
                assert_eq!(ranges.borrow().last().unwrap(), &scrolled, "redraw must preserve scroll position");
                println!("PASS: wheel moved rows {initial:?} -> {scrolled:?}; redraw preserved offset");
            }).expect("exercise rendered scrolling");
            cx.update(|cx| cx.quit());
        }).detach();
    });
}

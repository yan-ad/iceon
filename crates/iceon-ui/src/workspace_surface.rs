//! Keep outer dock chrome clear; the center owns an opaque content canvas.

use std::{rc::Rc, sync::Arc};

use gpui_kit::base::ResizeHandleContext;
use gpui_kit::component::dock::{
    BasePanelView, DockAreaRenderer, DockContext, DockSkin, DropIndicator, NodeId, PanelState,
    TabGroupContext, TabGroupRenderer,
};
use gpui_kit::*;

pub(super) struct MaterialDock(pub(super) Rc<DockSkin>);

fn content_background<T: Styled>(frame: T, background: Hsla) -> T {
    frame.bg(background)
}

fn clear_background<T: Styled>(mut frame: T) -> T {
    frame.style().background = Some(transparent_black().into());
    frame
}

impl DockAreaRenderer for MaterialDock {
    fn frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        clear_background(self.0.frame(window, cx))
    }

    fn center_frame(&self, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        content_background(
            self.0.center_frame(window, cx),
            gpui_kit::component::theme::Theme::global(cx).background,
        )
    }

    fn split_frame(
        &self,
        node: NodeId,
        axis: Axis,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        clear_background(self.0.split_frame(node, axis, window, cx))
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.render_split_handle(handle, window, cx)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.0.render_dock(dock, content, window, cx)
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn BasePanelView>> {
        self.0.build_placeholder(state, window, cx)
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(MaterialTabs(self.0.tab_group_renderer()))
    }
}

struct MaterialTabs(Rc<dyn TabGroupRenderer>);

impl TabGroupRenderer for MaterialTabs {
    fn frame(&self, group: &TabGroupContext, window: &mut Window, cx: &mut App) -> Stateful<Div> {
        // The stock tab-group frame paints theme.background even when its
        // title bar is hidden. The center canvas owns the content backing;
        // tab wrappers must not cover adjacent native-material chrome.
        clear_background(self.0.frame(group, window, cx))
    }

    fn content_frame(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        clear_background(self.0.content_frame(group, window, cx))
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.0.render_tab_bar(group, window, cx)
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.0.render_active_panel(panel, group, window, cx)
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.render_drop_indicator(indicator, window, cx)
    }

    fn render_empty(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.render_empty(group, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{clear_background, content_background};
    use gpui_kit::{InteractiveElement as _, Styled as _, div, rgb};

    #[test]
    fn center_canvas_paints_content_not_chrome_and_preserves_style() {
        let content = rgb(0x102030).into();
        let mut frame = content_background(
            div()
                .id("center-canvas-test")
                .border_r_1()
                .bg(rgb(0x405060)),
            content,
        );
        assert_eq!(
            frame.style().background.as_ref().unwrap().color(),
            Some(content.into())
        );
        assert_eq!(content.a, 1.0);
        assert!(frame.style().border_widths.right.is_some());
    }

    #[test]
    fn dock_backing_is_clear_without_losing_frame_style() {
        let mut frame = clear_background(
            div()
                .id("dock-surface-test")
                .bg(rgb(0x102030))
                .border_r_1()
                .text_color(rgb(0xabcdef)),
        );
        assert!(
            frame
                .style()
                .background
                .as_ref()
                .unwrap()
                .color()
                .unwrap()
                .is_transparent()
        );
        assert_eq!(frame.style().text_style().color, Some(rgb(0xabcdef).into()));
        assert!(frame.style().border_widths.right.is_some());
    }
}

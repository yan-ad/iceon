//! Native menu-bar status item (not a simulated GPUI tray).
//!
//! Hooks for the application shell:
//! - After opening the main window, call `register_main_window(handle.into(), cx)`.
//! - Apply the persisted `show_tray_icon` with `set_visible`, including on changes.
//! - Call `shutdown` from the app-quit hook, before GPUI is torn down.
//!
//! All hooks must run on the macOS main thread. The status item and its action
//! target are retained in thread-local storage, never in a `Send` global.

/// Whether this build has an actual native status-item implementation.
pub const fn supported() -> bool {
    cfg!(target_os = "macos")
}

/// Show or remove the native status item. Repeated calls are idempotent.
/// Enabling requires a registered main window; no preferences are written here.
pub fn set_visible(visible: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return native::set_visible(visible);
    #[cfg(not(target_os = "macos"))]
    if visible {
        Err("Native tray icons are not supported on this platform".into())
    } else {
        Ok(())
    }
}

/// Register the existing main window, not a preferences/dialog window.
/// This does not create a status item or bring a window forward.
pub fn register_main_window(
    window: gpui_kit::AnyWindowHandle,
    cx: &gpui_kit::App,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return native::register_main_window(window, cx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, cx);
        Ok(())
    }
}

/// Remove the item and forget the weak GPUI context before app teardown.
pub fn shutdown() -> Result<(), String> {
    set_visible(false)?;
    #[cfg(target_os = "macos")]
    native::clear_main_window();
    Ok(())
}

// Shared lifecycle logic is tested without creating an AppKit object or changing
// the user's menu bar. Failed construction leaves the old state untouched.
#[cfg(any(target_os = "macos", test))]
fn reconcile<T>(
    item: &mut Option<T>,
    visible: bool,
    create: impl FnOnce() -> Result<T, String>,
) -> Result<(), String> {
    if visible {
        if item.is_none() {
            *item = Some(create()?);
        }
    } else {
        *item = None;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use std::cell::RefCell;

    use gpui_kit::{AnyWindowHandle, App, AsyncApp};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel};
    use objc2_app_kit::{NSImage, NSSquareStatusItemLength, NSStatusBar, NSStatusItem};
    use objc2_foundation::{NSObject, NSObjectProtocol, NSSize, ns_string};

    #[derive(Clone)]
    struct MainWindow {
        window: AnyWindowHandle,
        cx: AsyncApp,
    }

    #[derive(Default)]
    struct State {
        main_window: Option<MainWindow>,
        item: Option<StatusItem>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    define_class!(
        // SAFETY: NSObject has no subclass invariants; all use is main-thread-only.
        #[unsafe(super = NSObject)]
        #[name = "IceonTrayActionTarget"]
        #[thread_kind = MainThreadOnly]
        struct ActionTarget;

        unsafe impl NSObjectProtocol for ActionTarget {}

        impl ActionTarget {
            // SAFETY: Matches AppKit target/action's void action(id) signature.
            #[unsafe(method(showIceon:))]
            fn show_iceon(&self, _sender: &NSObject) {
                // Release the TLS borrow before entering GPUI. update_window is
                // fallible if the handle closed, GPUI is quitting, or is borrowed.
                let main = STATE.with(|state| state.borrow().main_window.clone());
                if let Some(mut main) = main {
                    let _ = main.window.update(&mut main.cx, |_, window, cx| {
                        cx.activate(true);
                        window.activate_window();
                    });
                }
            }
        }
    );

    struct StatusItem {
        item: Retained<NSStatusItem>,
        // NSControl does not retain its target; keep it until after disconnection.
        _target: Retained<ActionTarget>,
        mtm: MainThreadMarker,
    }

    impl StatusItem {
        fn new(mtm: MainThreadMarker) -> Result<Self, String> {
            // Resolve the image before creating an item, so failure leaves no
            // empty menu-bar slot. SF Symbols are available on supported macOS.
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                ns_string!("cube"),
                Some(ns_string!("Iceon — Show main window")),
            )
            .ok_or_else(|| "macOS could not load the Iceon status icon".to_string())?;
            image.setTemplate(true);
            image.setSize(NSSize::new(18.0, 18.0));
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
            let Some(button) = item.button(mtm) else {
                NSStatusBar::systemStatusBar().removeStatusItem(&item);
                return Err("macOS did not provide a status-item button".into());
            };
            // SAFETY: NSObject's init is valid for this ivar-free subclass.
            let target: Retained<ActionTarget> =
                unsafe { msg_send![ActionTarget::alloc(mtm), init] };
            button.setImage(Some(&image));
            button.setToolTip(Some(ns_string!("Iceon — Show main window")));
            // SAFETY: target implements showIceon: with the correct signature and
            // is retained by this controller for the entire button lifetime.
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(showIceon:)));
            }
            Ok(Self {
                item,
                _target: target,
                mtm,
            })
        }
    }

    impl Drop for StatusItem {
        fn drop(&mut self) {
            if let Some(button) = self.item.button(self.mtm) {
                // SAFETY: Disconnect before releasing the non-retained target.
                unsafe {
                    button.setAction(None);
                    button.setTarget(None);
                }
            }
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }

    fn main_thread() -> Result<MainThreadMarker, String> {
        MainThreadMarker::new()
            .ok_or_else(|| "Tray updates must run on the macOS main thread".into())
    }

    pub(super) fn register_main_window(window: AnyWindowHandle, cx: &App) -> Result<(), String> {
        let _mtm = main_thread()?;
        STATE.with(|state| {
            state.borrow_mut().main_window = Some(MainWindow {
                window,
                cx: cx.to_async(),
            });
        });
        Ok(())
    }

    pub(super) fn clear_main_window() {
        STATE.with(|state| state.borrow_mut().main_window = None);
    }

    pub(super) fn set_visible(visible: bool) -> Result<(), String> {
        let mtm = main_thread()?;
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if visible && state.main_window.is_none() {
                return Err("Register the main GPUI window before enabling the tray icon".into());
            }
            super::reconcile(&mut state.item, visible, || StatusItem::new(mtm))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::reconcile;
    use std::{cell::Cell, rc::Rc};

    struct Item(Rc<Cell<usize>>);
    impl Drop for Item {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn repeated_show_hide_retains_once_and_removes_once() {
        let drops = Rc::new(Cell::new(0));
        let mut item = None;
        reconcile(&mut item, false, || panic!("hidden must not allocate")).unwrap();
        reconcile(&mut item, true, || Ok(Item(drops.clone()))).unwrap();
        reconcile(&mut item, true, || panic!("visible must not duplicate")).unwrap();
        assert_eq!(drops.get(), 0);
        reconcile(&mut item, false, || panic!("hide must not allocate")).unwrap();
        reconcile(&mut item, false, || panic!("hide must be idempotent")).unwrap();
        assert_eq!(drops.get(), 1);
        reconcile(&mut item, true, || Ok(Item(drops.clone()))).unwrap();
        drop(item);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn creation_failure_can_be_retried() {
        let mut item = None;
        assert!(reconcile::<u8>(&mut item, true, || Err("failure".into())).is_err());
        assert!(item.is_none());
        reconcile(&mut item, true, || Ok(1)).unwrap();
        assert_eq!(item, Some(1));
    }
}

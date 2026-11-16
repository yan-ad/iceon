//! Safe activation plumbing for the two public native actions.
//!
//! **Spotlight indexing is not enabled.** Core Spotlight selection is delivered
//! through `application:continueUserActivity:restorationHandler:`, not GPUI's
//! `on_open_urls`. The pinned gpui-pre-macos 0.3.11 delegate exposes openURLs but
//! no supported continuation callback. Replacing/swizzling that delegate is not
//! safe. `contentURL` is documented as a *file* URL (available since macOS 10.11),
//! not a supported custom-scheme launch override. Do not index inert items with
//! an `iceon://` contentURL and claim Spotlight activation works.
//!
//! Sources checked against Apple's documentation and the pinned upstream source:
//! - <https://developer.apple.com/documentation/corespotlight/cssearchableitemattributeset/contenturl>
//! - <https://developer.apple.com/library/archive/documentation/General/Conceptual/AppSearch/AppContent.html>
//! - <https://developer.apple.com/documentation/appkit/nsapplicationdelegate/application(_:continue:restorationhandler:)>
//! - <https://docs.rs/crate/gpui-pre-macos/0.3.11/source/src/platform.rs>
//!
//! Packaged macOS builds register `iceon` with Launch Services. Both cold and
//! running URL opens use GPUI's existing delegate. Only the exact public URLs
//! below are accepted; nothing is logged, persisted, executed, or provisioned.
//! Create Cluster means opening the existing kubeconfig import chooser. Settings
//! means opening the existing preferences window, without changing settings.
//! No cluster names, endpoints, paths, or credentials are sent to Spotlight.

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StaticAction {
    CreateCluster,
    Settings,
}

#[cfg(any(target_os = "macos", test))]
impl StaticAction {
    fn from_url(url: &str) -> Option<Self> {
        // Deliberately do not normalize or decode attacker-controlled input.
        // Even empty query/fragment/path suffixes and encoded aliases fail shut.
        match url {
            "iceon://create-cluster" => Some(Self::CreateCluster),
            "iceon://settings" => Some(Self::Settings),
            _ => None,
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use activation::UrlActivation;

#[cfg(target_os = "macos")]
mod activation {
    use std::{cell::RefCell, rc::Rc};

    use gpui_kit::{AnyWindowHandle, App, Application, AsyncApp, WeakEntity};

    use super::StaticAction;
    use crate::IceonApp;

    #[derive(Clone)]
    struct MainWindow {
        window: AnyWindowHandle,
        app: WeakEntity<IceonApp>,
    }

    #[derive(Default)]
    struct State {
        main: Option<MainWindow>,
        context: Option<AsyncApp>,
        // Keep only the latest valid request, never unbounded raw URL strings.
        pending: Option<StaticAction>,
    }

    /// Register before `Application::run` so launch-time URLs cannot be lost.
    pub(crate) struct UrlActivation(Rc<RefCell<State>>);

    impl UrlActivation {
        pub(crate) fn install(application: &Application) -> Self {
            let state = Rc::new(RefCell::new(State::default()));
            let callback_state = state.clone();
            let executor = application.foreground_executor();
            application.on_open_urls(move |urls| {
                let Some(action) = urls
                    .iter()
                    .rev()
                    .find_map(|url| StaticAction::from_url(url))
                else {
                    return;
                };
                callback_state.borrow_mut().pending = Some(action);
                let Some(context) = callback_state.borrow().context.clone() else {
                    return;
                };
                let state = callback_state.clone();
                // URL delivery can happen while AppKit/GPUI is dispatching an
                // event. Defer the update instead of reentering its App borrow.
                executor
                    .spawn(async move {
                        let _ = context.update(|cx| dispatch_pending(&state, cx));
                    })
                    .detach();
            });
            Self(state)
        }

        /// Bind only the shell, never whichever preferences/dialog has focus.
        /// Also drain the single request that may have arrived during launch.
        pub(crate) fn register_main_window(
            &self,
            window: AnyWindowHandle,
            app: WeakEntity<IceonApp>,
            cx: &mut App,
        ) {
            {
                let mut state = self.0.borrow_mut();
                state.main = Some(MainWindow { window, app });
                state.context = Some(cx.to_async());
            }
            dispatch_pending(&self.0, cx);
        }
    }

    fn dispatch_pending(state: &RefCell<State>, cx: &mut App) {
        let request = {
            let mut state = state.borrow_mut();
            let Some(main) = state.main.clone() else {
                return;
            };
            state.pending.take().map(|action| (main, action))
        };
        let Some((main, action)) = request else {
            return;
        };
        // A closed window or dropped entity fails harmlessly; it cannot target
        // another workspace or resurrect stale application state.
        let _ = main.window.update(cx, |_, window, cx| {
            let _ = main.app.update(cx, |app, cx| {
                cx.activate(true);
                window.activate_window();
                match action {
                    StaticAction::CreateCluster => app.open_add_config(window, cx),
                    StaticAction::Settings => app.open_preferences(window, cx),
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::StaticAction;

    #[test]
    fn only_two_exact_public_action_urls_are_accepted() {
        assert_eq!(
            StaticAction::from_url("iceon://create-cluster"),
            Some(StaticAction::CreateCluster)
        );
        assert_eq!(
            StaticAction::from_url("iceon://settings"),
            Some(StaticAction::Settings)
        );
    }

    #[test]
    fn urls_cannot_supply_paths_parameters_credentials_or_other_actions() {
        for url in [
            "",
            "settings",
            "ICEON://settings",
            "iceon://Settings",
            " iceon://settings",
            "iceon://settings\n",
            "iceon://settings/",
            "iceon://settings?",
            "iceon://settings?theme=dark",
            "iceon://settings#",
            "iceon://settings#tab",
            "iceon://settings:123",
            "iceon://user@settings",
            "iceon://settings.evil",
            "iceon://%73ettings",
            "iceon:///settings",
            "iceon:settings",
            "https://settings",
            "file:///settings",
            "iceon://create-cluster/",
            "iceon://create-cluster?path=fixture",
            "iceon://create-cluster?token=fixture",
            "iceon://create-cluster#tab",
            "iceon://create-cluster/../settings",
            "iceon://terminal",
            "iceon://settings\0",
            "iceon://settings%00",
            "iceon://settings\\",
        ] {
            assert_eq!(StaticAction::from_url(url), None, "accepted {url:?}");
        }
    }
}

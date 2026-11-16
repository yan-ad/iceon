//! Fixture-only preview; no user home discovery, authentication, or cluster startup.
use crate::IceonApp;
use gpui_kit::{
    component::{Theme, theme::ThemeMode},
    prelude::*,
    *,
};
use iceon_core::{
    catalog::CatalogEngine,
    connection::{ConnectionEngine, ConnectionError, diagnostics::ConnectionSetupDiagnostic},
    preferences::PreferencesEngine,
    secrets,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Preview {
    owner: Entity<IceonApp>,
    _fixture: Arc<Fixture>,
    _subscription: Subscription,
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |app, cx| app.render_connection_guide(cx))
    }
}

pub fn run() {
    let runtime = iceon_core::runtime::handle();
    let _guard = runtime.enter();
    let path = std::env::temp_dir().join(format!("iceon-guide-preview-{}", std::process::id()));
    std::fs::create_dir(&path).expect("fresh fixture directory");
    let fixture = Arc::new(Fixture(path));
    let missing = std::env::args().any(|arg| arg == "--missing-helper");
    let connecting = std::env::args().any(|arg| arg == "--connecting");
    let reviewing = std::env::args().any(|arg| arg == "--reviewing");
    let unavailable = std::env::args().any(|arg| arg == "--access-unavailable");
    let waiting = std::env::args().any(|arg| arg == "--waiting");
    let compact = std::env::args().any(|arg| arg == "--compact");
    let light = std::env::args().any(|arg| arg == "--light");
    gpui_kit::application()
        .with_assets(crate::app_assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            crate::console_font::init(cx);
            Theme::change(
                if light {
                    ThemeMode::Light
                } else {
                    ThemeMode::Dark
                },
                None,
                cx,
            );
            let fixture = fixture.clone();
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        if compact {
                            size(px(640.), px(540.))
                        } else {
                            size(px(1060.), px(780.))
                        },
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    window.set_window_title("Iceon connection guide — offline preview");
                    let catalog = Arc::new(CatalogEngine::new_with_home(fixture.0.clone(), None));
                    let connection = Arc::new(ConnectionEngine::new(catalog.clone()));
                    let preferences =
                        PreferencesEngine::new(fixture.0.clone()).expect("fixture preferences");
                    let secrets = secrets::Engine::new(fixture.0.clone());
                    let owner = cx.new(|cx| {
                        IceonApp::new(catalog, connection, preferences, secrets, window, cx)
                    });
                    if connecting || waiting {
                        owner.update(cx, |app, _| {
                            app.connection_guide.phase = if connecting {
                                crate::connection_guide::ConnectionPhase::Connecting
                            } else {
                                crate::connection_guide::ConnectionPhase::Waiting(
                                    ConnectionError::Network,
                                )
                            };
                        });
                    }
                    if missing {
                        owner.update(cx, |app, _| {
                            app.connection_guide.phase =
                                crate::connection_guide::ConnectionPhase::Failed(
                                    ConnectionError::AuthHelperMissing,
                                );
                            app.connection_guide.diagnostic = Some(ConnectionSetupDiagnostic {
                                context_name: None,
                                executable_basename: Some("gke-gcloud-auth-plugin".into()),
                            });
                        });
                    }
                    if reviewing || unavailable {
                        owner.update(cx, |app, _| {
                            app.history.push(crate::navigation::Route::Cluster {
                                cluster_id: "offline-guide".into(),
                                section: "overview".into(),
                            });
                            app.cluster_session =
                                Some(iceon_core::connection::ConnectionSnapshot {
                                    cluster_id: "offline-guide".into(),
                                    session_id: "offline-lease".into(),
                                    context_name: "offline".into(),
                                    status: "connected",
                                    api_version: None,
                                    error: None,
                                });
                            app.connection_guide.phase =
                                crate::connection_guide::ConnectionPhase::Connected;
                            app.fixture_guide_access_review(reviewing);
                            let saved = (app.sidebar_collapsed, app.cluster_sidebar_collapsed);
                            assert!(app.connection_guide_visible());
                            assert!(!app.cluster_sidebar_open());
                            assert_eq!(
                                saved,
                                (app.sidebar_collapsed, app.cluster_sidebar_collapsed)
                            );
                        });
                    }
                    if waiting {
                        // Only invalidate the isolated preview; never schedule a connection.
                        let deadline = std::time::Instant::now() + Duration::from_secs(5);
                        owner.update(cx, |app, cx| {
                            app.connection_guide.retry_deadline = Some(deadline);
                            app.connection_guide.recovery_task =
                                Some(cx.spawn(async move |owner, cx| {
                                    for _ in 0..6 {
                                        cx.background_executor()
                                            .timer(Duration::from_secs(1))
                                            .await;
                                        let _ = owner.update(cx, |_, cx| cx.notify());
                                    }
                                }));
                        });
                    }
                    cx.new(|cx| {
                        let subscription = cx.observe(&owner, |_, _, cx| cx.notify());
                        Preview {
                            owner,
                            _fixture: fixture,
                            _subscription: subscription,
                        }
                    })
                },
            )
            .expect("offline guide window");
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(12))
                    .await;
                cx.update(|cx| {
                    println!("PASS connection guide native render completed");
                    cx.quit();
                });
            })
            .detach();
            cx.activate(true);
        });
}

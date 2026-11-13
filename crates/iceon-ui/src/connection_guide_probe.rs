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
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |app, cx| app.render_connection_guide(false, cx))
    }
}

pub fn run() {
    let runtime = iceon_core::runtime::handle();
    let _guard = runtime.enter();
    let path = std::env::temp_dir().join(format!("iceon-guide-preview-{}", std::process::id()));
    std::fs::create_dir(&path).expect("fresh fixture directory");
    let fixture = Arc::new(Fixture(path));
    let missing = std::env::args().any(|arg| arg == "--missing-helper");
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
                    if missing {
                        owner.update(cx, |app, _| {
                            app.connection_guide.error = Some(ConnectionError::AuthHelperMissing);
                            app.connection_guide.diagnostic = Some(ConnectionSetupDiagnostic {
                                context_name: None,
                                executable_basename: Some("gke-gcloud-auth-plugin".into()),
                            });
                        });
                    }
                    cx.new(|_| Preview {
                        owner,
                        _fixture: fixture,
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

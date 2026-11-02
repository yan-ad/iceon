use super::*;
use tauri::State;

#[tauri::command]
pub async fn secrets_profile_update(
    engine: State<'_, Arc<Engine>>,
    request: ProfileUpdate,
) -> Result<Profile> {
    engine.update(request).await
}
#[tauri::command]
pub async fn secrets_profile_pin(
    engine: State<'_, Arc<Engine>>,
    profile_id: String,
    pinned: bool,
) -> Result<Profile> {
    engine.pin(profile_id, pinned).await
}
#[tauri::command]
pub async fn secrets_mounts(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: ReadRequest,
) -> Result<Mounts> {
    engine.mounts(window.label(), request).await
}
#[tauri::command]
pub async fn secrets_metadata(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: ReadRequest,
) -> Result<Metadata> {
    engine.metadata(window.label(), request).await
}

#[tauri::command]
pub async fn secrets_profiles(engine: State<'_, Arc<Engine>>) -> Result<Vec<Profile>> {
    engine.profiles().await
}
#[tauri::command]
pub async fn secrets_profile_save(
    engine: State<'_, Arc<Engine>>,
    request: ProfileInput,
) -> Result<Profile> {
    engine.save(request).await
}
#[tauri::command]
pub async fn secrets_profile_remove(
    engine: State<'_, Arc<Engine>>,
    profile_id: String,
) -> Result<()> {
    engine.remove(profile_id).await
}
#[tauri::command]
pub async fn secrets_session_open(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    profile_id: String,
    owner_id: String,
) -> Result<SessionView> {
    engine.open(window.label(), &profile_id, owner_id).await
}
#[tauri::command]
pub async fn secrets_login(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: LoginRequest,
) -> Result<SessionView> {
    engine.login(window.label(), request).await
}
#[tauri::command]
pub async fn secrets_resume(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: SessionRequest,
) -> Result<SessionView> {
    engine.resume(window.label(), request).await
}
#[tauri::command]
pub async fn secrets_disconnect(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: DisconnectRequest,
) -> Result<()> {
    engine.disconnect(window.label(), request).await
}
#[tauri::command]
pub async fn secrets_list(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: ReadRequest,
) -> Result<Listing> {
    engine.list(window.label(), request).await
}
#[tauri::command]
pub async fn secrets_reveal(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: ReadRequest,
) -> Result<Reveal> {
    engine.reveal(window.label(), request).await
}
#[tauri::command]
pub fn secrets_cancel(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: CancelRequest,
) -> Result<()> {
    engine.cancel(window.label(), request)
}
#[tauri::command]
pub fn secrets_close(
    window: tauri::WebviewWindow,
    engine: State<'_, Arc<Engine>>,
    request: SessionRequest,
) -> Result<()> {
    engine.close(window.label(), &request)
}

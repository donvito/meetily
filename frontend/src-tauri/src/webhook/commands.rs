use tauri::{AppHandle, Runtime};

use super::dispatch;
use super::logic::test_payload;
use super::session;
use super::settings::{self, WebhookSettings};

#[tauri::command]
pub async fn get_webhook_settings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<WebhookSettings, String> {
    settings::load_from_app(&app).await
}

#[tauri::command]
pub async fn set_webhook_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: WebhookSettings,
) -> Result<(), String> {
    let settings = settings::normalize(settings)?;
    let pool = settings::pool(&app)?;
    settings::save(&pool, &settings)
        .await
        .map_err(|err| format!("Failed to save webhook settings: {err}"))?;
    log::info!(
        "Saved meeting webhook settings (enabled={}, mode={})",
        settings.enabled,
        settings.mode
    );
    session::apply_settings(settings);
    Ok(())
}

/// Post a sample payload with reason "test". Uses the form values passed in,
/// so it can be tried before Save. Does not require a recording.
#[tauri::command]
pub async fn send_webhook_test<R: Runtime>(
    _app: AppHandle<R>,
    settings: WebhookSettings,
) -> Result<String, String> {
    let settings = settings::normalize(settings)?;
    if settings.url.is_empty() {
        return Err("Enter a webhook URL first".to_string());
    }
    let status = dispatch::post(&settings, &test_payload()).await?;
    Ok(format!("Test payload delivered (HTTP {status})"))
}

#[tauri::command]
pub async fn get_last_recording_meeting_id() -> Option<String> {
    session::last_meeting_id()
}

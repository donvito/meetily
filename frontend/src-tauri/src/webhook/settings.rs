// Webhook preferences live in the local SQLite database, including the sender
// key. Provider API keys are stored in that same database.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

pub const DEFAULT_HEADER_NAME: &str = "Authorization";
pub const DEFAULT_WAKE_WORD: &str = "Emma";
pub const DEFAULT_INTERVAL_MINUTES: u32 = 5;
const MAX_INTERVAL_MINUTES: u32 = 24 * 60;

#[derive(Clone, Serialize, Deserialize)]
pub struct WebhookSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default = "default_header_name")]
    pub header_name: String,
    #[serde(default)]
    pub header_value: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
    #[serde(default = "default_wake_word")]
    pub wake_word: String,
}

fn default_header_name() -> String {
    DEFAULT_HEADER_NAME.to_string()
}

fn default_mode() -> String {
    "wake_word".to_string()
}

fn default_interval() -> u32 {
    DEFAULT_INTERVAL_MINUTES
}

fn default_wake_word() -> String {
    DEFAULT_WAKE_WORD.to_string()
}

impl Default for WebhookSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            header_name: default_header_name(),
            header_value: String::new(),
            mode: default_mode(),
            interval_minutes: default_interval(),
            wake_word: default_wake_word(),
        }
    }
}

impl std::fmt::Debug for WebhookSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookSettings")
            .field("enabled", &self.enabled)
            .field("has_url", &!self.url.is_empty())
            .field("header_name", &self.header_name)
            .field("header_value", &"<redacted>")
            .field("mode", &self.mode)
            .field("interval_minutes", &self.interval_minutes)
            .field("wake_word", &self.wake_word)
            .finish()
    }
}

#[derive(sqlx::FromRow)]
struct WebhookRow {
    enabled: i64,
    url: String,
    header_name: String,
    header_value: String,
    mode: String,
    interval_minutes: i64,
    wake_word: String,
}

impl From<WebhookRow> for WebhookSettings {
    fn from(row: WebhookRow) -> Self {
        Self {
            enabled: row.enabled != 0,
            url: row.url,
            header_name: row.header_name,
            header_value: row.header_value,
            mode: row.mode,
            interval_minutes: row.interval_minutes.max(0) as u32,
            wake_word: row.wake_word,
        }
    }
}

pub fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "Database is not initialized".to_string())?;
    Ok(state.db_manager.pool().clone())
}

pub async fn load(pool: &SqlitePool) -> Result<WebhookSettings, sqlx::Error> {
    let row = sqlx::query_as::<_, WebhookRow>(
        r#"SELECT enabled, url, header_name, header_value, mode, interval_minutes, wake_word
           FROM webhook_settings WHERE id = '1'"#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(WebhookSettings::from).unwrap_or_default())
}

pub async fn save(pool: &SqlitePool, settings: &WebhookSettings) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO webhook_settings
            (id, enabled, url, header_name, header_value, mode, interval_minutes, wake_word)
           VALUES ('1', ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(id) DO UPDATE SET
            enabled = excluded.enabled,
            url = excluded.url,
            header_name = excluded.header_name,
            header_value = excluded.header_value,
            mode = excluded.mode,
            interval_minutes = excluded.interval_minutes,
            wake_word = excluded.wake_word"#,
    )
    .bind(if settings.enabled { 1 } else { 0 })
    .bind(&settings.url)
    .bind(&settings.header_name)
    .bind(&settings.header_value)
    .bind(&settings.mode)
    .bind(settings.interval_minutes as i64)
    .bind(&settings.wake_word)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_from_app<R: Runtime>(app: &AppHandle<R>) -> Result<WebhookSettings, String> {
    let pool = pool(app)?;
    load(&pool)
        .await
        .map_err(|err| format!("Failed to load webhook settings: {err}"))
}

/// Trim surrounding whitespace. The header value is otherwise sent unchanged:
/// a pasted `Bearer ...` value keeps that prefix, and a raw token does not gain one.
pub fn normalize(mut settings: WebhookSettings) -> Result<WebhookSettings, String> {
    settings.url = settings.url.trim().to_string();
    settings.header_name = settings.header_name.trim().to_string();
    if settings.header_name.is_empty() {
        settings.header_name = DEFAULT_HEADER_NAME.to_string();
    }
    settings.header_value = settings.header_value.trim().to_string();
    settings.wake_word = settings.wake_word.trim().to_string();
    if settings.wake_word.is_empty() {
        settings.wake_word = DEFAULT_WAKE_WORD.to_string();
    }
    settings.mode = settings.mode.trim().to_ascii_lowercase();
    if settings.mode != "wake_word" && settings.mode != "interval" {
        return Err("Mode must be wake_word or interval".to_string());
    }
    if settings.interval_minutes < 1 {
        settings.interval_minutes = 1;
    } else if settings.interval_minutes > MAX_INTERVAL_MINUTES {
        settings.interval_minutes = MAX_INTERVAL_MINUTES;
    }
    if !settings.url.is_empty() {
        validate_url(&settings.url)?;
    }
    validate_header_name(&settings.header_name)?;
    if !settings.header_value.is_empty() {
        validate_header_value(&settings.header_value)?;
    }
    if settings.wake_word.chars().count() > 64 {
        return Err("Wake word must be 64 characters or fewer".to_string());
    }
    Ok(settings)
}

pub fn validate_url(url: &str) -> Result<(), String> {
    if url.len() > 2048 {
        return Err("Webhook URL is too long".to_string());
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| "Webhook URL is not valid".to_string())?;
    if parsed.scheme() != "https" && parsed.scheme() != "http" {
        return Err("Webhook URL must start with https:// or http://".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("Webhook URL must include a host".to_string());
    }
    Ok(())
}

fn validate_header_name(name: &str) -> Result<(), String> {
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map(|_| ())
        .map_err(|_| "Auth header name is not a valid HTTP header name".to_string())
}

fn validate_header_value(value: &str) -> Result<(), String> {
    reqwest::header::HeaderValue::from_str(value)
        .map(|_| ())
        .map_err(|_| "Auth header value contains invalid characters".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keeps_a_pasted_header_value_without_adding_bearer() {
        let settings = normalize(WebhookSettings {
            enabled: true,
            url: " https://example.com/hook ".to_string(),
            header_name: " Authorization ".to_string(),
            header_value: " rout_abc ".to_string(),
            mode: "WAKE_WORD".to_string(),
            interval_minutes: 0,
            wake_word: " Emma ".to_string(),
        })
        .unwrap();
        assert_eq!(settings.url, "https://example.com/hook");
        assert_eq!(settings.header_name, "Authorization");
        assert_eq!(settings.header_value, "rout_abc");
        assert!(!settings.header_value.starts_with("Bearer "));
        assert_eq!(settings.mode, "wake_word");
        assert_eq!(settings.interval_minutes, 1);
        assert_eq!(settings.wake_word, "Emma");
    }

    #[test]
    fn normalize_preserves_an_explicit_bearer_prefix() {
        let settings = normalize(WebhookSettings {
            header_value: "Bearer pasted-token".to_string(),
            ..WebhookSettings::default()
        })
        .unwrap();
        assert_eq!(settings.header_value, "Bearer pasted-token");
    }

    #[test]
    fn normalize_rejects_a_non_http_url() {
        let error = normalize(WebhookSettings {
            url: "file:///tmp/hook".to_string(),
            ..WebhookSettings::default()
        })
        .unwrap_err();
        assert!(error.contains("https://"));
    }

    #[tokio::test]
    async fn settings_round_trip_stores_the_sender_key() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(include_str!(
            "../../migrations/20261022120000_add_webhook_settings.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let defaults = load(&pool).await.unwrap();
        assert!(!defaults.enabled);
        assert_eq!(defaults.header_name, "Authorization");
        assert_eq!(defaults.wake_word, "Emma");
        assert_eq!(defaults.interval_minutes, 5);
        assert!(defaults.header_value.is_empty());

        let saved = WebhookSettings {
            enabled: true,
            url: "https://example.com/hook".to_string(),
            header_name: "Authorization".to_string(),
            header_value: "rout_secret".to_string(),
            mode: "interval".to_string(),
            interval_minutes: 12,
            wake_word: "Ada".to_string(),
        };
        save(&pool, &saved).await.unwrap();
        let loaded = load(&pool).await.unwrap();
        assert!(loaded.enabled);
        assert_eq!(loaded.url, saved.url);
        assert_eq!(loaded.header_value, "rout_secret");
        assert_eq!(loaded.mode, "interval");
        assert_eq!(loaded.interval_minutes, 12);
        assert_eq!(loaded.wake_word, "Ada");
    }
}

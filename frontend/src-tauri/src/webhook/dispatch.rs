// One HTTP POST. Short timeout, no retries, no redirect following.
// Redirects are refused so the auth header is not forwarded to another host.

use std::time::Duration;

use crate::webhook::logic::WebhookPayload;
use crate::webhook::settings::WebhookSettings;

pub const POST_TIMEOUT: Duration = Duration::from_secs(5);

pub fn endpoint_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(|host| host.to_string()))
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Header name and value exactly as stored. Does not add a scheme prefix.
pub fn auth_header(settings: &WebhookSettings) -> Result<Option<(String, String)>, String> {
    let name = settings.header_name.trim();
    let value = settings.header_value.trim();
    if name.is_empty() || value.is_empty() {
        return Ok(None);
    }
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| "Auth header name is not a valid HTTP header name".to_string())?;
    reqwest::header::HeaderValue::from_str(value)
        .map_err(|_| "Auth header value contains invalid characters".to_string())?;
    Ok(Some((name.to_string(), value.to_string())))
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(POST_TIMEOUT)
        .connect_timeout(POST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Failed to build HTTP client".to_string())
}

/// Post `payload` once. The error string is safe to show and to log: it has the
/// status or a host, and it does not include the header value or response body.
pub async fn post(settings: &WebhookSettings, payload: &WebhookPayload) -> Result<u16, String> {
    let url = settings.url.trim();
    if url.is_empty() {
        return Err("Webhook URL is empty".to_string());
    }
    crate::webhook::settings::validate_url(url)?;
    let host = endpoint_host(url);

    let client = client()?;
    let mut request = client.post(url).json(payload);
    if let Some((name, value)) = auth_header(settings)? {
        request = request.header(name, value);
    }

    let response = request.send().await.map_err(|err| {
        if err.is_timeout() {
            format!("Webhook request to {host} timed out after 5 seconds")
        } else {
            format!("Webhook request to {host} failed")
        }
    })?;

    let status = response.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else if status.is_redirection() {
        Err(format!(
            "Webhook host {host} returned HTTP {status}; redirects are not followed"
        ))
    } else {
        Err(format!("Webhook host {host} returned HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhook::settings::WebhookSettings;

    #[test]
    fn auth_header_is_the_pasted_value() {
        let settings = WebhookSettings {
            header_name: "Authorization".to_string(),
            header_value: "rout_abc".to_string(),
            ..WebhookSettings::default()
        };
        let (name, value) = auth_header(&settings).unwrap().unwrap();
        assert_eq!(name, "Authorization");
        assert_eq!(value, "rout_abc");
        assert!(!value.to_lowercase().starts_with("bearer "));
    }

    #[test]
    fn auth_header_keeps_a_pasted_bearer_prefix() {
        let settings = WebhookSettings {
            header_name: "Authorization".to_string(),
            header_value: "Bearer pasted-token".to_string(),
            ..WebhookSettings::default()
        };
        let (_, value) = auth_header(&settings).unwrap().unwrap();
        assert_eq!(value, "Bearer pasted-token");
    }

    #[test]
    fn empty_sender_key_omits_the_header() {
        let settings = WebhookSettings::default();
        assert!(auth_header(&settings).unwrap().is_none());
    }

    #[test]
    fn endpoint_host_hides_userinfo_and_path() {
        assert_eq!(
            endpoint_host("https://user:secret@example.com/hook?token=abc"),
            "example.com"
        );
    }
}

//! Spawn safety guards — rate limiter + anomalous spawn alerting.
//!
//! Extracted from spawn_routes.rs to stay under 300-line limit.

use std::collections::HashMap;
use std::time::Instant;

use serde_json::json;
use tokio::sync::Mutex;

/// Max spawns per org per sliding window.
pub const RATE_LIMIT_MAX: usize = 10;
/// Sliding window duration in seconds.
pub const RATE_LIMIT_WINDOW_SECS: u64 = 60;
/// Alert threshold: fire Telegram if this many spawns in ALERT_WINDOW_SECS.
const ALERT_THRESHOLD: usize = 5;
const ALERT_WINDOW_SECS: u64 = 600;

/// Sliding-window rate limiter: org_id → list of spawn timestamps.
pub type RateLimiter = Mutex<HashMap<String, Vec<Instant>>>;

/// Create a new empty rate limiter.
pub fn new_rate_limiter() -> RateLimiter {
    Mutex::new(HashMap::new())
}

/// Check rate limit for an org. Returns Ok(spawn_count_in_alert_window) or
/// Err(error_message) if rate limit exceeded.
pub async fn check_rate_limit(limiter: &RateLimiter, org_id: &str) -> Result<usize, String> {
    let mut guard = limiter.lock().await;
    let now = Instant::now();
    let window = std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);
    let timestamps = guard.entry(org_id.to_string()).or_default();
    timestamps.retain(|t| now.duration_since(*t) < window);

    if timestamps.len() >= RATE_LIMIT_MAX {
        return Err(format!(
            "rate limit: max {} spawns per {}s for org {}",
            RATE_LIMIT_MAX, RATE_LIMIT_WINDOW_SECS, org_id
        ));
    }
    timestamps.push(now);

    // Count recent spawns in alert window
    let alert_window = std::time::Duration::from_secs(ALERT_WINDOW_SECS);
    let recent = timestamps
        .iter()
        .filter(|t| now.duration_since(**t) < alert_window)
        .count();
    Ok(recent)
}

/// Check if alert threshold is met and fire Telegram alert if so.
pub fn maybe_fire_alert(org_id: &str, recent_count: usize) {
    if recent_count >= ALERT_THRESHOLD {
        let org = org_id.to_string();
        tokio::spawn(async move {
            fire_spawn_alert(&org, recent_count).await;
        });
    }
}

/// Send Telegram alert for anomalous spawn rate.
async fn fire_spawn_alert(org_id: &str, count: usize) {
    let Ok(client) = telegram_client_from_env() else {
        tracing::debug!("spawn alert: Telegram not configured, skipping");
        return;
    };
    let text = format!(
        "🚨 <b>Anomalous spawn rate</b>\n\n\
         Org: <code>{org_id}</code>\n\
         Spawns in last 10min: <b>{count}</b>\n\n\
         Check daemon logs and consider pausing the plan."
    );
    if let Err(e) = client.send(&text).await {
        tracing::warn!("spawn alert Telegram failed: {e}");
    }
}

fn telegram_client_from_env() -> Result<TelegramClient, String> {
    let bot_token = std::env::var("CONVERGIO_TELEGRAM_BOT_TOKEN")
        .map_err(|_| "CONVERGIO_TELEGRAM_BOT_TOKEN not set".to_string())?;
    let chat_id = std::env::var("CONVERGIO_TELEGRAM_CHAT_ID")
        .map_err(|_| "CONVERGIO_TELEGRAM_CHAT_ID not set".to_string())?;
    Ok(TelegramClient { bot_token, chat_id })
}

struct TelegramClient {
    bot_token: String,
    chat_id: String,
}

impl TelegramClient {
    async fn send(&self, text: &str) -> Result<(), String> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let payload = json!({
            "chat_id": self.chat_id,
            "text": text,
            "parse_mode": "HTML"
        });
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("Telegram API error: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("Telegram API returned {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rate_limit_allows_under_threshold() {
        let limiter = new_rate_limiter();
        let result = check_rate_limit(&limiter, "test-org").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
    }

    #[tokio::test]
    async fn rate_limit_blocks_over_threshold() {
        let limiter = new_rate_limiter();
        for _ in 0..RATE_LIMIT_MAX {
            check_rate_limit(&limiter, "test-org").await.unwrap();
        }
        let result = check_rate_limit(&limiter, "test-org").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("rate limit"));
    }

    #[tokio::test]
    async fn rate_limit_is_per_org() {
        let limiter = new_rate_limiter();
        for _ in 0..RATE_LIMIT_MAX {
            check_rate_limit(&limiter, "org-a").await.unwrap();
        }
        // Different org should still be allowed
        let result = check_rate_limit(&limiter, "org-b").await;
        assert!(result.is_ok());
    }
}

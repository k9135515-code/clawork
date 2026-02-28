use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use clawork_core::{InboundMessage, MessageAdapter, OutboundMessage, SendResult};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct TelegramAdapter {
    client: Client,
    bot_token: String,
    offset: Arc<Mutex<i64>>,
}

#[derive(Deserialize)]
struct TelegramResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
}

#[derive(Deserialize)]
struct TelegramMessage {
    text: Option<String>,
    chat: TelegramChat,
}

#[derive(Deserialize)]
struct TelegramChat {
    id: i64,
}

impl TelegramAdapter {
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            bot_token: bot_token.into(),
            offset: Arc::new(Mutex::new(0)),
        }
    }

    fn base_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.bot_token, method)
    }
}

#[async_trait]
impl MessageAdapter for TelegramAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        let offset = *self.offset.lock().await;
        let resp = self
            .client
            .get(self.base_url("getUpdates"))
            .query(&[("offset", offset.to_string())])
            .send()
            .await
            .context("telegram getUpdates request")?;

        let body: TelegramResponse = resp.json().await.context("telegram parse response")?;
        if !body.ok {
            return Ok(vec![]);
        }

        let mut max_seen = offset;
        let items = body
            .result
            .into_iter()
            .filter_map(|update| {
                if update.update_id >= max_seen {
                    max_seen = update.update_id + 1;
                }
                let message = update.message?;
                Some(InboundMessage {
                    adapter: "telegram".into(),
                    from: message.chat.id.to_string(),
                    content: message.text.unwrap_or_default(),
                    received_at: Utc::now(),
                    external_id: Some(update.update_id.to_string()),
                })
            })
            .collect();
        *self.offset.lock().await = max_seen;
        Ok(items)
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let payload = serde_json::json!({
            "chat_id": message.to,
            "text": message.content,
        });

        let resp = self
            .client
            .post(self.base_url("sendMessage"))
            .json(&payload)
            .send()
            .await
            .context("telegram sendMessage request")?;

        if resp.status().is_success() {
            Ok(SendResult {
                ok: true,
                provider_id: None,
                error: None,
            })
        } else {
            Ok(SendResult {
                ok: false,
                provider_id: None,
                error: Some(format!("telegram returned status {}", resp.status())),
            })
        }
    }
}

#[derive(Clone)]
pub struct WhatsAppCloudAdapter {
    client: Client,
    access_token: String,
    phone_number_id: String,
}

impl WhatsAppCloudAdapter {
    pub fn new(access_token: impl Into<String>, phone_number_id: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            access_token: access_token.into(),
            phone_number_id: phone_number_id.into(),
        }
    }

    fn messages_url(&self) -> String {
        format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        )
    }
}

#[async_trait]
impl MessageAdapter for WhatsAppCloudAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        let path = std::env::var("CLAWORK_WHATSAPP_POLL_FILE")
            .unwrap_or_else(|_| "data/whatsapp_inbound.jsonl".to_string());
        let file_path = std::path::Path::new(&path);
        if !file_path.exists() {
            return Ok(vec![]);
        }

        let raw = tokio::fs::read_to_string(file_path)
            .await
            .with_context(|| format!("read whatsapp poll file: {}", file_path.display()))?;
        if raw.trim().is_empty() {
            return Ok(vec![]);
        }

        let mut inbound = Vec::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let from = v
                .get("from")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let content = v
                .get("content")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            if from.is_empty() || content.is_empty() {
                continue;
            }
            let external_id = v
                .get("external_id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string);

            inbound.push(InboundMessage {
                adapter: "whatsapp".into(),
                from,
                content,
                received_at: Utc::now(),
                external_id,
            });
        }

        let _ = tokio::fs::remove_file(file_path).await;
        Ok(inbound)
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": message.to,
            "type": "text",
            "text": { "body": message.content }
        });

        let resp = self
            .client
            .post(self.messages_url())
            .bearer_auth(&self.access_token)
            .json(&payload)
            .send()
            .await
            .context("whatsapp cloud send request")?;

        if resp.status().is_success() {
            Ok(SendResult {
                ok: true,
                provider_id: None,
                error: None,
            })
        } else {
            Ok(SendResult {
                ok: false,
                provider_id: None,
                error: Some(format!("whatsapp returned status {}", resp.status())),
            })
        }
    }
}

#[derive(Clone)]
pub struct LineAdapter {
    client: Client,
    channel_access_token: String,
}

impl LineAdapter {
    pub fn new(channel_access_token: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            channel_access_token: channel_access_token.into(),
        }
    }
}

#[async_trait]
impl MessageAdapter for LineAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        Ok(vec![])
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let payload = serde_json::json!({
            "to": message.to,
            "messages": [{"type": "text", "text": message.content}]
        });

        let resp = self
            .client
            .post("https://api.line.me/v2/bot/message/push")
            .bearer_auth(&self.channel_access_token)
            .json(&payload)
            .send()
            .await
            .context("line push request")?;

        Ok(status_result("line", resp.status().as_u16(), None))
    }
}

#[derive(Clone)]
pub struct DiscordAdapter {
    client: Client,
    webhook_url: String,
}

impl DiscordAdapter {
    pub fn new(webhook_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            webhook_url: webhook_url.into(),
        }
    }
}

#[async_trait]
impl MessageAdapter for DiscordAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        Ok(vec![])
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let content = if message.to.is_empty() {
            message.content
        } else {
            format!("{}\n(to: {})", message.content, message.to)
        };

        let resp = self
            .client
            .post(&self.webhook_url)
            .json(&serde_json::json!({"content": content}))
            .send()
            .await
            .context("discord webhook send")?;

        Ok(status_result("discord", resp.status().as_u16(), None))
    }
}

#[derive(Clone)]
pub struct SlackAdapter {
    client: Client,
    webhook_url: Option<String>,
    bot_token: Option<String>,
    poll_channels: Arc<Vec<String>>,
    latest_ts_by_channel: Arc<Mutex<HashMap<String, f64>>>,
}

impl SlackAdapter {
    pub fn new(
        webhook_url: Option<String>,
        bot_token: Option<String>,
        poll_channels: Vec<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            webhook_url: webhook_url
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            bot_token: bot_token
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            poll_channels: Arc::new(
                poll_channels
                    .into_iter()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect(),
            ),
            latest_ts_by_channel: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn send_enabled(&self) -> bool {
        self.webhook_url.is_some()
    }
}

#[async_trait]
impl MessageAdapter for SlackAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        let Some(bot_token) = &self.bot_token else {
            return Ok(vec![]);
        };
        if self.poll_channels.is_empty() {
            return Ok(vec![]);
        }

        let mut inbound = Vec::new();
        for channel in self.poll_channels.iter() {
            let oldest = {
                let guard = self.latest_ts_by_channel.lock().await;
                guard.get(channel).copied().unwrap_or(0.0)
            };

            let resp = self
                .client
                .get("https://slack.com/api/conversations.history")
                .bearer_auth(bot_token)
                .query(&[
                    ("channel", channel.as_str()),
                    ("limit", "50"),
                    ("inclusive", "false"),
                    ("oldest", &oldest.to_string()),
                ])
                .send()
                .await
                .context("slack conversations.history request")?;

            if !resp.status().is_success() {
                continue;
            }

            let body: Value = resp
                .json()
                .await
                .context("slack conversations.history parse")?;
            if body.get("ok").and_then(Value::as_bool) != Some(true) {
                continue;
            }

            let mut max_seen = oldest;
            let messages = body
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for message in messages.into_iter().rev() {
                if message
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    != "message"
                {
                    continue;
                }

                let ts = message
                    .get("ts")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if ts.is_empty() {
                    continue;
                }
                let ts_num = ts.parse::<f64>().unwrap_or(0.0);
                if ts_num <= oldest {
                    continue;
                }
                if ts_num > max_seen {
                    max_seen = ts_num;
                }

                let content = message
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if content.is_empty() {
                    continue;
                }

                let from = message
                    .get("user")
                    .and_then(Value::as_str)
                    .or_else(|| message.get("bot_id").and_then(Value::as_str))
                    .unwrap_or("unknown")
                    .to_string();

                inbound.push(InboundMessage {
                    adapter: "slack".into(),
                    from,
                    content,
                    received_at: Utc::now(),
                    external_id: Some(format!("{channel}:{ts}")),
                });
            }

            let mut guard = self.latest_ts_by_channel.lock().await;
            if max_seen > oldest {
                guard.insert(channel.clone(), max_seen);
            }
        }

        Ok(inbound)
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let Some(webhook_url) = &self.webhook_url else {
            return Err(anyhow::anyhow!("slack webhook is not configured"));
        };
        let text = if message.to.is_empty() {
            message.content
        } else {
            format!("{}\n(to: {})", message.content, message.to)
        };

        let resp = self
            .client
            .post(webhook_url)
            .json(&serde_json::json!({"text": text}))
            .send()
            .await
            .context("slack webhook send")?;

        Ok(status_result("slack", resp.status().as_u16(), None))
    }
}

#[derive(Clone)]
pub struct SignalAdapter {
    command: String,
    from_number: String,
}

impl SignalAdapter {
    pub fn new(command: impl Into<String>, from_number: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            from_number: from_number.into(),
        }
    }
}

#[async_trait]
impl MessageAdapter for SignalAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        Ok(vec![])
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        let output = Command::new(&self.command)
            .args([
                "-a",
                &self.from_number,
                "send",
                "-m",
                &message.content,
                &message.to,
            ])
            .output()
            .await
            .with_context(|| format!("signal-cli command failed: {}", self.command))?;

        if output.status.success() {
            Ok(SendResult {
                ok: true,
                provider_id: None,
                error: None,
            })
        } else {
            Ok(SendResult {
                ok: false,
                provider_id: None,
                error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
            })
        }
    }
}

#[derive(Clone)]
pub struct IMessageAdapter;

#[async_trait]
impl MessageAdapter for IMessageAdapter {
    async fn poll(&self) -> anyhow::Result<Vec<InboundMessage>> {
        Ok(vec![])
    }

    async fn send(&self, message: OutboundMessage) -> anyhow::Result<SendResult> {
        #[cfg(target_os = "macos")]
        {
            let escaped_content = message.content.replace('"', "\\\"");
            let escaped_to = message.to.replace('"', "\\\"");
            let script = format!(
                "tell application \"Messages\" to send \"{}\" to buddy \"{}\"",
                escaped_content, escaped_to
            );

            let output = Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .await
                .context("osascript send failed")?;

            if output.status.success() {
                return Ok(SendResult {
                    ok: true,
                    provider_id: None,
                    error: None,
                });
            }

            return Ok(SendResult {
                ok: false,
                provider_id: None,
                error: Some(String::from_utf8_lossy(&output.stderr).to_string()),
            });
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = message;
            Ok(SendResult {
                ok: false,
                provider_id: None,
                error: Some("iMessage adapter is only available on macOS".into()),
            })
        }
    }
}

fn status_result(provider: &str, status: u16, provider_id: Option<String>) -> SendResult {
    let ok = (200..300).contains(&status);
    if ok {
        SendResult {
            ok,
            provider_id,
            error: None,
        }
    } else {
        SendResult {
            ok,
            provider_id,
            error: Some(format!("{provider} returned status {status}")),
        }
    }
}

//! Outbound-only Twilio ingress: poll the REST API for inbound messages instead
//! of receiving a webhook. This lets the stage run behind a locked-down network
//! (no public URL, no tunnel) — the app only ever makes outbound HTTPS calls to
//! `api.twilio.com`.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::TwilioPollConfig;

pub struct TwilioClient {
    http: reqwest::Client,
    account_sid: String,
    username: String,
    password: String,
    number: String,
}

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub sid: String,
    pub body: String,
}

#[derive(Deserialize)]
struct MessagesPage {
    messages: Vec<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    sid: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    direction: Option<String>,
}

impl TwilioClient {
    pub fn from_config(config: &TwilioPollConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()?;
        Ok(Self {
            http,
            account_sid: config.account_sid.clone(),
            username: config.auth_username.clone(),
            password: config.auth_password.clone(),
            number: config.number.clone(),
        })
    }

    /// Fetch recent inbound messages addressed to the configured number. Only
    /// mobile-originated (`direction == "inbound"`) messages with visible text
    /// are returned; the caller deduplicates by `sid`.
    pub async fn fetch_inbound(&self) -> Result<Vec<InboundMessage>> {
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.account_sid
        );
        let response = self
            .http
            .get(url)
            .basic_auth(&self.username, Some(&self.password))
            .query(&[("To", self.number.as_str()), ("PageSize", "50")])
            .send()
            .await
            .context("Twilio Messages request failed")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Twilio Messages returned HTTP {status}: {body}");
        }

        let page: MessagesPage = response
            .json()
            .await
            .context("decoding Twilio Messages response")?;

        Ok(page
            .messages
            .into_iter()
            .filter(|message| message.direction.as_deref() == Some("inbound"))
            .filter_map(|message| {
                let body = message.body.unwrap_or_default();
                (!body.trim().is_empty()).then_some(InboundMessage {
                    sid: message.sid,
                    body,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_page_defaults_missing_body_and_direction() {
        // `fetch_inbound` relies on these `#[serde(default)]` fields: the Twilio
        // Messages API omits `body`/`direction` on some rows, and a missing field
        // must deserialize to `None` rather than failing the whole page.
        let page: MessagesPage = serde_json::from_str(
            r#"{"messages":[
                {"sid":"SM1","body":"hello","direction":"inbound"},
                {"sid":"SM2"}
            ]}"#,
        )
        .unwrap();

        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].body.as_deref(), Some("hello"));
        assert_eq!(page.messages[0].direction.as_deref(), Some("inbound"));
        assert_eq!(page.messages[1].body, None);
        assert_eq!(page.messages[1].direction, None);
    }
}

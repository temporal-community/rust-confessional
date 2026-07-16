use std::{env, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    Fixture,
    OpenAi,
}

impl ModelProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixture => "fixture",
            Self::OpenAi => "openai",
        }
    }
}

impl FromStr for ModelProvider {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fixture" => Ok(Self::Fixture),
            "openai" => Ok(Self::OpenAi),
            other => bail!("unsupported MODEL_PROVIDER {other:?}; expected fixture or openai"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemporalConfig {
    pub task_queue: String,
}

impl TemporalConfig {
    pub fn from_env() -> Self {
        Self {
            task_queue: env_string("TEMPORAL_TASK_QUEUE", "rust-confessional"),
        }
    }
}

#[derive(Clone)]
pub struct StageConfig {
    pub bind_address: SocketAddr,
    pub data_path: PathBuf,
    pub static_dir: PathBuf,
    pub internal_token: String,
    pub max_confession_chars: usize,
    pub max_submissions_per_session: usize,
    pub show_raw_confessions: bool,
    pub twilio: Option<TwilioInboundConfig>,
    pub temporal: TemporalConfig,
    /// Operator-supplied words blanked on the stage projection. Sourced from
    /// `MASK_WORDS`; kept out of the repository so no word list is bundled.
    pub mask_words: Vec<String>,
}

#[derive(Clone)]
pub struct TwilioInboundConfig {
    pub account_sid: String,
    pub auth_token: String,
    pub webhook_url: String,
}

impl StageConfig {
    pub fn from_env() -> Result<Self> {
        let twilio = TwilioInboundConfig::from_env()?;
        let show_raw_confessions = env_parse("SHOW_RAW_CONFESSIONS", false)?;
        let allow_unmoderated_twilio = env_parse("ALLOW_UNMODERATED_TWILIO", false)?;
        if twilio.is_some() && show_raw_confessions && !allow_unmoderated_twilio {
            bail!(
                "SHOW_RAW_CONFESSIONS with Twilio requires explicit ALLOW_UNMODERATED_TWILIO=true"
            );
        }
        Ok(Self {
            bind_address: env_string("BIND_ADDRESS", "127.0.0.1:3000")
                .parse()
                .context("BIND_ADDRESS must be a socket address such as 127.0.0.1:3000")?,
            data_path: env_string("STAGE_DATA_PATH", "data/stage.json").into(),
            static_dir: env_string("STATIC_DIR", "static").into(),
            internal_token: env_string("STAGE_INTERNAL_TOKEN", "local-demo-token"),
            max_confession_chars: env_parse("MAX_CONFESSION_CHARS", 500usize)?,
            max_submissions_per_session: env_parse("MAX_SUBMISSIONS_PER_SESSION", 20usize)?,
            show_raw_confessions,
            twilio,
            temporal: TemporalConfig::from_env(),
            mask_words: parse_mask_words(&env_string("MASK_WORDS", "")),
        })
    }
}

/// Parse the `MASK_WORDS` list. Words may be separated by commas or any
/// whitespace, and are lowercased so matching in `moderation` stays consistent.
/// Masking is whole-word over alphanumeric runs, so entries containing other
/// characters (hyphens, apostrophes, or multi-word phrases) could never match;
/// those are dropped rather than stored as silent no-ops.
fn parse_mask_words(raw: &str) -> Vec<String> {
    raw.split(|character: char| character == ',' || character.is_whitespace())
        .filter(|word| !word.is_empty() && word.chars().all(char::is_alphanumeric))
        .map(str::to_lowercase)
        .collect()
}

impl TwilioInboundConfig {
    fn from_env() -> Result<Option<Self>> {
        let account_sid = nonempty_env("TWILIO_ACCOUNT_SID");
        let auth_token = nonempty_env("TWILIO_AUTH_TOKEN");
        let webhook_url = nonempty_env("TWILIO_WEBHOOK_URL");

        match (account_sid, auth_token, webhook_url) {
            (None, None, None) => Ok(None),
            (Some(account_sid), Some(auth_token), Some(webhook_url)) => {
                let parsed = url::Url::parse(&webhook_url)
                    .context("TWILIO_WEBHOOK_URL must be an absolute public URL")?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    bail!("TWILIO_WEBHOOK_URL must be an absolute HTTP(S) URL");
                }
                Ok(Some(Self {
                    account_sid,
                    auth_token,
                    webhook_url,
                }))
            }
            _ => bail!(
                "TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN, and TWILIO_WEBHOOK_URL must be set together"
            ),
        }
    }
}

#[derive(Clone)]
pub struct WorkerConfig {
    pub temporal: TemporalConfig,
    pub stage_internal_url: String,
    pub stage_internal_token: String,
    pub model_provider: ModelProvider,
    pub openai_api_key: Option<String>,
    pub openai_model: String,
    pub model_timeout: Duration,
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self> {
        let model_provider = env_string("MODEL_PROVIDER", "fixture").parse()?;
        let openai_api_key = env::var("OPENAI_API_KEY")
            .ok()
            .filter(|value| !value.is_empty());
        if model_provider == ModelProvider::OpenAi && openai_api_key.is_none() {
            bail!("OPENAI_API_KEY is required when MODEL_PROVIDER=openai");
        }

        Ok(Self {
            temporal: TemporalConfig::from_env(),
            stage_internal_url: env_string(
                "STAGE_INTERNAL_URL",
                "http://127.0.0.1:3000/api/internal",
            )
            .trim_end_matches('/')
            .to_owned(),
            stage_internal_token: env_string("STAGE_INTERNAL_TOKEN", "local-demo-token"),
            model_provider,
            openai_api_key,
            openai_model: env_string("OPENAI_MODEL", "gpt-5.6-luna"),
            model_timeout: Duration::from_secs(env_parse("MODEL_TIMEOUT_SECONDS", 12u64)?),
        })
    }
}

fn env_string(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_parse<T>(name: &str, default: T) -> Result<T>
where
    T: FromStr + Copy,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("invalid value for {name}")),
        Err(_) => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parser_is_explicit() {
        assert_eq!(
            "fixture".parse::<ModelProvider>().unwrap(),
            ModelProvider::Fixture
        );
        assert!("automatic".parse::<ModelProvider>().is_err());
    }
}

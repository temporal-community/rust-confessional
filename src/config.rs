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
    /// Outbound-only polling of Twilio's REST API. An alternative to the inbound
    /// webhook for hosts that cannot expose a public URL (e.g. a locked-down
    /// laptop): the app pulls new messages instead of Twilio pushing them.
    pub twilio_poll: Option<TwilioPollConfig>,
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
        let twilio_poll = TwilioPollConfig::from_env()?;
        let show_raw_confessions = env_parse("SHOW_RAW_CONFESSIONS", false)?;
        let allow_unmoderated_twilio = env_parse("ALLOW_UNMODERATED_TWILIO", false)?;
        if (twilio.is_some() || twilio_poll.is_some())
            && show_raw_confessions
            && !allow_unmoderated_twilio
        {
            bail!(
                "SHOW_RAW_CONFESSIONS with Twilio requires explicit ALLOW_UNMODERATED_TWILIO=true"
            );
        }
        Ok(Self {
            bind_address: env_string("BIND_ADDRESS", "127.0.0.1:3000")
                .parse()
                .context("BIND_ADDRESS must be a socket address such as 127.0.0.1:3000")?,
            // Fixed internal paths (resolved against the container WORKDIR `/app`,
            // where the `stage-data` volume and baked-in `static/` live). Kept out
            // of the environment so no untrusted value can reach a filesystem call
            // — the data path is plumbing, not an operator knob.
            data_path: PathBuf::from("data/stage.json"),
            static_dir: PathBuf::from("static"),
            internal_token: env_string("STAGE_INTERNAL_TOKEN", "local-demo-token"),
            max_confession_chars: env_parse("MAX_CONFESSION_CHARS", 500usize)?,
            max_submissions_per_session: env_parse("MAX_SUBMISSIONS_PER_SESSION", 20usize)?,
            show_raw_confessions,
            twilio,
            twilio_poll,
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
    /// The inbound webhook is keyed on `TWILIO_WEBHOOK_URL`: it activates only
    /// when that URL is present. `TWILIO_ACCOUNT_SID` alone no longer implies
    /// webhook mode, because the API poller reuses it without a public URL.
    fn from_env() -> Result<Option<Self>> {
        let Some(webhook_url) = nonempty_env("TWILIO_WEBHOOK_URL") else {
            return Ok(None);
        };
        let account_sid = nonempty_env("TWILIO_ACCOUNT_SID")
            .context("TWILIO_WEBHOOK_URL requires TWILIO_ACCOUNT_SID")?;
        let auth_token = nonempty_env("TWILIO_AUTH_TOKEN")
            .context("TWILIO_WEBHOOK_URL requires TWILIO_AUTH_TOKEN")?;

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
}

/// Credentials and cadence for polling Twilio's REST API for inbound messages.
#[derive(Clone)]
pub struct TwilioPollConfig {
    pub account_sid: String,
    /// HTTP basic-auth username: an API key SID (`SK…`) when available, else the
    /// account SID paired with the auth token.
    pub auth_username: String,
    pub auth_password: String,
    /// The Twilio number to watch, in E.164; polled as the `To` of inbound texts.
    pub number: String,
    pub poll_interval: Duration,
}

impl TwilioPollConfig {
    /// Polling activates when `TWILIO_NUMBER` is set. Credentials prefer an API
    /// key (`TWILIO_API_KEY_SID` + `TWILIO_API_KEY_SECRET`) and fall back to
    /// `TWILIO_AUTH_TOKEN`, matching Twilio's own recommendation to avoid the
    /// account auth token where possible.
    fn from_env() -> Result<Option<Self>> {
        let Some(number) = nonempty_env("TWILIO_NUMBER") else {
            return Ok(None);
        };
        let account_sid = nonempty_env("TWILIO_ACCOUNT_SID")
            .context("TWILIO_NUMBER requires TWILIO_ACCOUNT_SID for API polling")?;

        let (auth_username, auth_password) = match (
            nonempty_env("TWILIO_API_KEY_SID"),
            nonempty_env("TWILIO_API_KEY_SECRET"),
            nonempty_env("TWILIO_AUTH_TOKEN"),
        ) {
            (Some(key_sid), Some(key_secret), _) => (key_sid, key_secret),
            (_, _, Some(auth_token)) => (account_sid.clone(), auth_token),
            _ => bail!(
                "Twilio polling needs TWILIO_API_KEY_SID + TWILIO_API_KEY_SECRET (preferred) or TWILIO_AUTH_TOKEN"
            ),
        };

        let poll_seconds: u64 = env_parse("TWILIO_POLL_SECONDS", 4u64)?;
        Ok(Some(Self {
            account_sid,
            auth_username,
            auth_password,
            number,
            poll_interval: Duration::from_secs(poll_seconds.max(1)),
        }))
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

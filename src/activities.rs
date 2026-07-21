use std::{sync::Arc, time::Duration};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use temporalio_macros::activities;
use temporalio_sdk::{
    ApplicationFailure,
    activities::{ActivityContext, ActivityError},
};
use tokio::time::sleep;
use tracing::warn;

use crate::{
    agent::{AgentBackend, ModelError, remedy_for},
    domain::{AgentPlan, Judgment, Remedy, StageUpdate, SubmissionInput, SubmissionStatus},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupInput {
    pub plan: AgentPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeInput {
    pub submission: SubmissionInput,
    pub plan: AgentPlan,
    pub remedy: Option<Remedy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryInput {
    pub submission_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub submission_id: String,
    pub channel: String,
}

pub struct ConfessionalActivities {
    backend: Arc<dyn AgentBackend>,
    reporter: StageReporter,
}

impl ConfessionalActivities {
    pub fn new(
        backend: Arc<dyn AgentBackend>,
        stage_internal_url: String,
        stage_internal_token: String,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            backend,
            reporter: StageReporter::new(stage_internal_url, stage_internal_token)?,
        })
    }

    pub fn model_mode(&self) -> &'static str {
        self.backend.mode()
    }
}

#[activities]
impl ConfessionalActivities {
    #[activity]
    pub async fn plan(
        self: Arc<Self>,
        _ctx: ActivityContext,
        submission: SubmissionInput,
    ) -> Result<AgentPlan, ActivityError> {
        self.backend.plan(&submission).await.map_err(model_error)
    }

    #[activity]
    pub async fn lookup_remedy(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: LookupInput,
    ) -> Result<Remedy, ActivityError> {
        // This is deliberately an Activity even though the demo catalog is bundled. In a
        // production agent it can become an approved docs/crates lookup without changing
        // deterministic Workflow code.
        sleep(Duration::from_millis(450)).await;
        Ok(remedy_for(input.plan.category))
    }

    #[activity]
    pub async fn compose(
        self: Arc<Self>,
        ctx: ActivityContext,
        input: ComposeInput,
    ) -> Result<Judgment, ActivityError> {
        // Stage fault injection: a confession that mentions rate limiting makes the
        // model call fail transiently on its first two attempts, so the audience can
        // watch Temporal retry with backoff and recover on its own (high-level
        // reliability) while the typed retryable error is Rust's job (low-level).
        let attempt = ctx.info().attempt;
        if simulates_rate_limit(&input.submission.text) && attempt <= 2 {
            warn!(
                attempt,
                "injecting simulated model rate limit for the stage demo"
            );
            return Err(ActivityError::application(ApplicationFailure::new(
                format!("simulated model rate limit (HTTP 429) on attempt {attempt}"),
            )));
        }
        self.backend
            .compose(&input.submission, &input.plan, input.remedy.as_ref())
            .await
            .map_err(model_error)
    }

    #[activity]
    pub async fn report_stage(
        self: Arc<Self>,
        _ctx: ActivityContext,
        update: StageUpdate,
    ) -> Result<(), ActivityError> {
        self.reporter
            .send_update(&update)
            .await
            .map_err(|error| ActivityError::application(ApplicationFailure::new(error.to_string())))
    }

    #[activity]
    pub async fn deliver(
        self: Arc<Self>,
        _ctx: ActivityContext,
        input: DeliveryInput,
    ) -> Result<DeliveryReceipt, ActivityError> {
        // The stage build uses the dashboard as the delivery channel. A real SMS adapter
        // belongs here and must deduplicate by submission.id because Activities are at least once.
        // Stay in Sending long enough for the 700 ms projector poll to show recovery.
        sleep(Duration::from_millis(1_200)).await;
        Ok(DeliveryReceipt {
            submission_id: input.submission_id,
            channel: "stage".to_owned(),
        })
    }
}

#[derive(Clone)]
pub struct StageReporter {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl StageReporter {
    pub fn new(base_url: String, token: String) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(4))
                .build()?,
            base_url,
            token,
        })
    }

    pub async fn send_update(&self, update: &StageUpdate) -> anyhow::Result<()> {
        let response = self
            .client
            .post(format!("{}/update", self.base_url))
            .bearer_auth(&self.token)
            .json(update)
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("stage update returned HTTP {}", response.status())
        }
    }

    pub async fn heartbeat(&self, model_mode: &str) -> anyhow::Result<()> {
        let response = self
            .client
            .post(format!("{}/worker-heartbeat", self.base_url))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "model_mode": model_mode }))
            .send()
            .await?;
        if response.status() == StatusCode::NO_CONTENT || response.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("stage heartbeat returned HTTP {}", response.status())
        }
    }

    pub async fn heartbeat_loop(self, model_mode: &'static str) {
        loop {
            if let Err(error) = self.heartbeat(model_mode).await {
                warn!(%error, "could not report Worker heartbeat");
            }
            sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Stage-only trigger: confessions that mention rate limiting opt into a
/// simulated transient model outage so the retry-and-recover beat is
/// deterministic and needs no live, flaky API.
fn simulates_rate_limit(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "rate limit",
        "rate-limit",
        "ratelimit",
        "429",
        "too many requests",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn model_error(error: ModelError) -> ActivityError {
    if error.is_retryable() {
        ActivityError::application(ApplicationFailure::new(error.to_string()))
    } else {
        ActivityError::application(ApplicationFailure::non_retryable(error.to_string()))
    }
}

pub fn stage_update(
    submission: &SubmissionInput,
    status: SubmissionStatus,
    judgment: Option<Judgment>,
) -> StageUpdate {
    StageUpdate {
        id: submission.id.clone(),
        session_id: submission.session_id.clone(),
        status,
        judgment,
        error: None,
    }
}

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use temporalio_client::Client;
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock},
    time::sleep,
};
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing::{info, warn};
use ulid::Ulid;

use crate::{
    config::{StageConfig, TwilioPollConfig},
    domain::{
        AgentMode, Awards, PublicStageState, StageSubmission, StageUpdate, SubmissionInput,
        SubmissionStatus, WorkflowMode,
    },
    temporal,
    twilio::{compliance_keyword, form_field, parse_form_body, verify_twilio_signature},
    twilio_poll::TwilioClient,
};

const SEED_CONFESSIONS: &[&str] = &[
    "I fixed the race condition with a sleep.",
    "I clone everything until it compiles.",
    "I wrote a Python script that now runs the company.",
];

#[derive(Clone)]
pub struct StageApp {
    store: Arc<StageStore>,
    temporal: Arc<TemporalGateway>,
    heartbeat: Arc<RwLock<WorkerHeartbeat>>,
    admission: Arc<Mutex<()>>,
    config: StageConfig,
}

impl StageApp {
    pub async fn new(config: StageConfig) -> anyhow::Result<Self> {
        let store = Arc::new(StageStore::load(config.data_path.clone()).await?);
        let temporal = Arc::new(TemporalGateway::new(config.temporal.task_queue.clone()));
        Ok(Self {
            store,
            temporal,
            heartbeat: Arc::new(RwLock::new(WorkerHeartbeat::default())),
            admission: Arc::new(Mutex::new(())),
            config,
        })
    }

    pub fn router(self) -> Router {
        let static_dir = self.config.static_dir.clone();
        Router::new()
            .route("/healthz", get(health))
            .route("/api/state", get(get_state))
            .route("/api/confessions", post(submit_confession))
            .route("/webhooks/twilio/messages", post(twilio_message))
            .route("/api/demo/hold", post(set_hold))
            .route("/api/demo/mode", post(set_workflow_mode))
            .route("/api/demo/agent-mode", post(set_agent_mode))
            .route("/api/demo/seed", post(seed_confessions))
            .route("/api/demo/reset", post(reset_demo))
            .route("/api/internal/update", post(internal_update))
            .route("/api/internal/worker-heartbeat", post(worker_heartbeat))
            .fallback_service(ServeDir::new(static_dir).append_index_html_on_directories(true))
            .layer(TraceLayer::new_for_http())
            .with_state(self)
    }

    async fn submit_text(
        &self,
        text: String,
        requested_id: Option<String>,
    ) -> Result<StageSubmission, ApiError> {
        let _admission = self.admission.lock().await;
        self.submit_text_admitted(text, requested_id).await
    }

    async fn submit_text_admitted(
        &self,
        text: String,
        requested_id: Option<String>,
    ) -> Result<StageSubmission, ApiError> {
        let text = text.trim().to_owned();
        let char_count = text.chars().count();
        if char_count == 0 {
            return Err(ApiError::bad_request("confession cannot be empty"));
        }
        if char_count > self.config.max_confession_chars {
            return Err(ApiError::bad_request(format!(
                "confession is {char_count} characters; maximum is {}",
                self.config.max_confession_chars
            )));
        }

        // Reject input that is empty once control characters and surrounding
        // whitespace are removed. Emoji and punctuation count as content.
        let cleaned = crate::moderation::sanitize_for_stage(&text);
        if cleaned.is_empty() {
            return Err(ApiError::bad_request(
                "confession must contain visible text",
            ));
        }

        let (session_id, held) = self.store.session_and_hold().await;
        let mode = self.store.mode().await;
        // Each confession captures the currently selected agent mode; linear and
        // autonomous confessions coexist in one session (no reset on change).
        let agent_mode = self.store.agent_mode().await;
        let id = requested_id.unwrap_or_else(|| Ulid::new().to_string());
        validate_submission_id(&id)?;
        let mut input = SubmissionInput {
            id,
            session_id,
            text,
            created_at: Utc::now(),
            hold_before_reply: held,
            agent_mode,
        };
        let workflow_id = match mode {
            WorkflowMode::PerConfession => temporal::workflow_id(&input.id),
            WorkflowMode::Session => temporal::session_workflow_id(&input.session_id),
        };
        let mut staged = StageSubmission::received(&input, workflow_id.clone());
        if self.config.show_raw_confessions {
            // Raw display: show the audience's own words, but sanitized to the
            // stage character set and with any operator-listed words masked.
            staged.text = crate::moderation::mask_words(&cleaned, &self.config.mask_words);
        }
        let (staged, inserted) = self
            .store
            .insert_if_absent(staged, self.config.max_submissions_per_session)
            .await
            .map_err(|error| match error {
                InsertError::AtCapacity => ApiError::too_many_requests(format!(
                    "this session has reached its limit of {} confessions",
                    self.config.max_submissions_per_session
                )),
                InsertError::Persistence(error) => ApiError::internal(error),
            })?;

        // Use the first accepted row's stable metadata on source retries. Temporal's
        // Workflow ID remains the authoritative idempotency boundary.
        input.session_id.clone_from(&staged.session_id);
        input.created_at = staged.created_at;

        if !inserted
            && !matches!(
                staged.status,
                SubmissionStatus::Received | SubmissionStatus::Failed
            )
        {
            return Ok(staged);
        }

        let client = match self.temporal.client().await {
            Ok(client) => client,
            Err(error) => {
                self.store
                    .mark_failed(&input.id, error.to_string())
                    .await
                    .map_err(ApiError::internal)?;
                return Err(ApiError::unavailable("Temporal is not connected yet"));
            }
        };

        let queue = &self.config.temporal.task_queue;
        let dispatch = match mode {
            WorkflowMode::PerConfession => {
                temporal::start_submission(&client, queue, input.clone())
                    .await
                    .map(|_| ())
            }
            WorkflowMode::Session => {
                // Ensure the session Workflow exists, then Signal this confession in.
                match temporal::start_session(&client, queue, &input.session_id).await {
                    Ok(session_workflow_id) => {
                        temporal::add_session_confession(
                            &client,
                            &session_workflow_id,
                            input.clone(),
                            &format!("add-{}", input.id),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
        };

        if let Err(error) = dispatch {
            self.temporal.mark_disconnected();
            self.store
                .mark_failed(&input.id, error.to_string())
                .await
                .map_err(ApiError::internal)?;
            return Err(ApiError::unavailable(format!(
                "Temporal did not accept the confession: {error}"
            )));
        }

        self.temporal.mark_connected();
        Ok(staged)
    }

    async fn seed_examples(&self) -> Result<usize, ApiError> {
        let _admission = self.admission.lock().await;
        let mut accepted = 0;
        for (index, confession) in SEED_CONFESSIONS.iter().enumerate() {
            self.submit_text_admitted((*confession).to_owned(), Some(format!("seed-{index}")))
                .await?;
            accepted += 1;
        }
        Ok(accepted)
    }

    async fn release_current(&self) -> Result<usize, ApiError> {
        let workflows = self.store.releasable_workflows().await;
        if workflows.is_empty() {
            return Ok(0);
        }
        let client = self
            .temporal
            .client()
            .await
            .map_err(|_| ApiError::unavailable("Temporal is not connected yet"))?;

        if self.store.mode().await == WorkflowMode::Session {
            // One aggregate Workflow: a single release Signal frees the whole session.
            let session_id = self.store.session_id().await;
            let workflow_id = temporal::session_workflow_id(&session_id);
            return match temporal::release_session(
                &client,
                &workflow_id,
                &format!("release-{session_id}"),
            )
            .await
            {
                Ok(()) => {
                    self.temporal.mark_connected();
                    Ok(workflows.len())
                }
                Err(error) => {
                    self.temporal.mark_disconnected();
                    warn!(%workflow_id, %error, "could not release session Workflow");
                    Err(ApiError::unavailable(
                        "release signal failed; retry the control",
                    ))
                }
            };
        }

        let mut released = 0;
        let mut failed = 0;
        for (submission_id, workflow_id) in workflows {
            let request_id = format!("release-{submission_id}");
            match temporal::release_submission(&client, &workflow_id, &request_id).await {
                Ok(()) => released += 1,
                Err(error) => {
                    failed += 1;
                    warn!(%workflow_id, %error, "could not release Workflow");
                }
            }
        }
        if failed > 0 {
            self.temporal.mark_disconnected();
            return Err(ApiError::unavailable(format!(
                "released {released} confessions, but {failed} release signals failed; retry the control"
            )));
        }
        self.temporal.mark_connected();
        Ok(released)
    }

    async fn wait_until_terminal(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.store.releasable_workflows().await.is_empty() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            sleep(Duration::from_millis(100)).await;
        }
    }
}

pub async fn run(config: StageConfig) -> anyhow::Result<()> {
    let bind_address = config.bind_address;
    let poll_config = config.twilio_poll.clone();
    let app = StageApp::new(config).await?;
    let gateway = app.temporal.clone();
    tokio::spawn(async move { gateway.connection_loop().await });

    if let Some(poll_config) = poll_config {
        let poller_app = app.clone();
        tokio::spawn(async move { run_twilio_poller(poller_app, poll_config).await });
    }

    let listener = TcpListener::bind(bind_address).await?;
    info!(%bind_address, "stage dashboard listening");
    axum::serve(listener, app.router())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Poll Twilio for inbound messages and feed new ones into the same submission
/// path as the browser form. The first successful fetch only *baselines* the
/// existing backlog (so texts sent before the demo started are ignored); after
/// that, each newly seen `MessageSid` becomes one confession exactly once.
async fn run_twilio_poller(app: StageApp, config: TwilioPollConfig) {
    let client = match TwilioClient::from_config(&config) {
        Ok(client) => client,
        Err(error) => {
            warn!(%error, "Twilio poller disabled: could not build client");
            return;
        }
    };
    info!(
        number = %config.number,
        interval_secs = config.poll_interval.as_secs(),
        "Twilio message polling enabled"
    );

    let mut seen: HashSet<String> = HashSet::new();
    let mut baselined = false;

    loop {
        match client.fetch_inbound().await {
            Ok(messages) => {
                for message in messages {
                    // `insert` returns false when the sid was already handled.
                    if !seen.insert(message.sid.clone()) {
                        continue;
                    }
                    // First pass records the pre-demo backlog without submitting it.
                    if !baselined {
                        continue;
                    }
                    // STOP/START/HELP are carrier-compliance commands, not confessions.
                    if compliance_keyword(&message.body).is_some() {
                        continue;
                    }
                    if let Err(error) = app
                        .submit_text(
                            message.body.clone(),
                            Some(format!("twilio-{}", message.sid)),
                        )
                        .await
                    {
                        warn!(reason = %error.1, "could not submit polled Twilio confession");
                    }
                }
                baselined = true;
            }
            Err(error) => warn!(%error, "Twilio poll failed; will retry next interval"),
        }
        sleep(config.poll_interval).await;
    }
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn get_state(State(app): State<StageApp>) -> Json<PublicStageState> {
    let heartbeat = app.heartbeat.read().await;
    let worker_online = heartbeat
        .last_seen
        .is_some_and(|last_seen| last_seen.elapsed() < Duration::from_secs(3));
    let model_mode = heartbeat.model_mode.clone();
    drop(heartbeat);

    let stored = app.store.snapshot().await;
    let awards = awards_for(&stored.submissions);
    Json(PublicStageState {
        worker_online,
        temporal_connected: app.temporal.is_connected(),
        model_mode,
        held: stored.held,
        show_raw_confessions: app.config.show_raw_confessions,
        workflow_mode: stored.workflow_mode,
        agent_mode: stored.agent_mode,
        submissions: stored.submissions,
        awards,
    })
}

#[derive(Debug, Deserialize)]
struct SubmitRequest {
    text: String,
}

async fn submit_confession(
    State(app): State<StageApp>,
    headers: HeaderMap,
    Json(request): Json<SubmitRequest>,
) -> Result<(StatusCode, Json<StageSubmission>), ApiError> {
    let requested_id = optional_idempotency_key(&headers)?.map(|key| format!("web-{key}"));
    let submission = app.submit_text(request.text, requested_id).await?;
    Ok((StatusCode::ACCEPTED, Json(submission)))
}

async fn twilio_message(
    State(app): State<StageApp>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let config = app
        .config
        .twilio
        .clone()
        .ok_or_else(|| ApiError::not_found("Twilio ingress is not configured"))?;

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type
        .to_ascii_lowercase()
        .starts_with("application/x-www-form-urlencoded")
    {
        return Err(ApiError::bad_request(
            "expected application/x-www-form-urlencoded",
        ));
    }

    let supplied_signature = headers
        .get("x-twilio-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::forbidden("invalid Twilio signature"))?;
    let fields = parse_form_body(&body);
    if !verify_twilio_signature(
        &config.auth_token,
        &config.webhook_url,
        &fields,
        supplied_signature,
    ) {
        return Err(ApiError::forbidden("invalid Twilio signature"));
    }

    let account_sid = required_form_field(&fields, "AccountSid")?;
    let message_sid = required_form_field(&fields, "MessageSid")?;
    let confession = required_form_field(&fields, "Body")?;
    // Validate the standard sender/recipient fields without retaining or logging them.
    let _ = required_form_field(&fields, "From")?;
    let _ = required_form_field(&fields, "To")?;
    if account_sid != config.account_sid {
        return Err(ApiError::forbidden("unexpected Twilio account"));
    }

    if compliance_keyword(confession).is_none() {
        app.submit_text(confession.to_owned(), Some(format!("twilio-{message_sid}")))
            .await?;
    }

    Ok(empty_twiml())
}

#[derive(Debug, Deserialize)]
struct HoldRequest {
    held: bool,
}

#[derive(Debug, Serialize)]
struct HoldResponse {
    held: bool,
    released: usize,
}

async fn set_hold(
    State(app): State<StageApp>,
    Json(request): Json<HoldRequest>,
) -> Result<Json<HoldResponse>, ApiError> {
    let _admission = app.admission.lock().await;
    app.store
        .set_held(request.held)
        .await
        .map_err(ApiError::internal)?;
    let released = if request.held {
        0
    } else {
        match app.release_current().await {
            Ok(released) => released,
            Err(error) => {
                // Keep new submissions at the checkpoint and make another false toggle an
                // obvious retry. Signals already accepted by Temporal are idempotent.
                app.store.set_held(true).await.map_err(ApiError::internal)?;
                return Err(error);
            }
        }
    };
    Ok(Json(HoldResponse {
        held: request.held,
        released,
    }))
}

#[derive(Debug, Serialize)]
struct SeedResponse {
    accepted: usize,
}

async fn seed_confessions(State(app): State<StageApp>) -> Result<Json<SeedResponse>, ApiError> {
    let accepted = app.seed_examples().await?;
    Ok(Json(SeedResponse { accepted }))
}

#[derive(Debug, Deserialize)]
struct ModeRequest {
    mode: WorkflowMode,
}

async fn set_workflow_mode(
    State(app): State<StageApp>,
    Json(request): Json<ModeRequest>,
) -> Result<StatusCode, ApiError> {
    let _admission = app.admission.lock().await;
    if app.store.mode().await == request.mode {
        return Ok(StatusCode::NO_CONTENT);
    }
    // Reset on switch: drain the current session, then start a clean one in the
    // new mode so the two architectures never interleave on stage.
    app.release_current().await?;
    if !app.wait_until_terminal(Duration::from_secs(12)).await {
        return Err(ApiError::conflict(
            "unfinished confessions are still draining; wait and switch again",
        ));
    }
    app.store.reset().await.map_err(ApiError::internal)?;
    app.store
        .set_mode(request.mode)
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct AgentModeRequest {
    agent_mode: AgentMode,
}

async fn set_agent_mode(
    State(app): State<StageApp>,
    Json(request): Json<AgentModeRequest>,
) -> Result<StatusCode, ApiError> {
    // Unlike the per/aggregate workflow mode, the agent mode does not reset the
    // session: each confession carries its own `agent_mode`, so linear and
    // autonomous confessions can coexist in one running session.
    let _admission = app.admission.lock().await;
    app.store
        .set_agent_mode(request.agent_mode)
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_demo(State(app): State<StageApp>) -> Result<StatusCode, ApiError> {
    let _admission = app.admission.lock().await;
    app.release_current().await?;
    if !app.wait_until_terminal(Duration::from_secs(12)).await {
        return Err(ApiError::conflict(
            "unfinished confessions are still draining; wait and reset again",
        ));
    }
    app.store.reset().await.map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn internal_update(
    State(app): State<StageApp>,
    headers: HeaderMap,
    Json(update): Json<StageUpdate>,
) -> Result<StatusCode, ApiError> {
    authorize_internal(&app, &headers)?;
    app.store
        .apply_update(update, !app.config.show_raw_confessions)
        .await
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct HeartbeatRequest {
    model_mode: String,
}

async fn worker_heartbeat(
    State(app): State<StageApp>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatRequest>,
) -> Result<StatusCode, ApiError> {
    authorize_internal(&app, &headers)?;
    let mut heartbeat = app.heartbeat.write().await;
    heartbeat.last_seen = Some(Instant::now());
    heartbeat.model_mode = request.model_mode;
    Ok(StatusCode::NO_CONTENT)
}

fn authorize_internal(app: &StageApp, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = format!("Bearer {}", app.config.internal_token);
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if supplied == Some(expected.as_str()) {
        Ok(())
    } else {
        Err(ApiError(
            StatusCode::UNAUTHORIZED,
            "unauthorized".to_owned(),
        ))
    }
}

#[derive(Debug, Default)]
struct WorkerHeartbeat {
    last_seen: Option<Instant>,
    model_mode: String,
}

struct TemporalGateway {
    client: RwLock<Option<Arc<Client>>>,
    connected: AtomicBool,
    task_queue: String,
}

impl TemporalGateway {
    fn new(task_queue: String) -> Self {
        Self {
            client: RwLock::new(None),
            connected: AtomicBool::new(false),
            task_queue,
        }
    }

    async fn client(&self) -> anyhow::Result<Arc<Client>> {
        if let Some(client) = self.client.read().await.clone() {
            return Ok(client);
        }
        let connected = Arc::new(temporal::connect_client().await?);
        *self.client.write().await = Some(connected.clone());
        self.mark_connected();
        Ok(connected)
    }

    async fn connection_loop(&self) {
        loop {
            if self.client.read().await.is_none() {
                match self.client().await {
                    Ok(_) => info!(task_queue = %self.task_queue, "connected stage to Temporal"),
                    Err(error) => {
                        self.mark_disconnected();
                        warn!(%error, "waiting for Temporal");
                    }
                }
            }
            sleep(Duration::from_secs(2)).await;
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn mark_connected(&self) {
        self.connected.store(true, Ordering::Relaxed);
    }

    fn mark_disconnected(&self) {
        self.connected.store(false, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredStageState {
    session_id: String,
    held: bool,
    #[serde(default)]
    workflow_mode: WorkflowMode,
    #[serde(default)]
    agent_mode: AgentMode,
    submissions: Vec<StageSubmission>,
}

impl Default for StoredStageState {
    fn default() -> Self {
        Self {
            session_id: Ulid::new().to_string(),
            held: true,
            workflow_mode: WorkflowMode::default(),
            // The demo defaults to the autonomous agent loop. `AgentMode::default()`
            // stays `Linear` for SubmissionInput replay safety; only the fresh store
            // state opts into autonomous so a clean session starts in the loop.
            agent_mode: AgentMode::Autonomous,
            submissions: Vec::new(),
        }
    }
}

struct StageStore {
    path: PathBuf,
    state: RwLock<StoredStageState>,
    persistence: Mutex<()>,
}

impl StageStore {
    async fn load(path: PathBuf) -> anyhow::Result<Self> {
        let state = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                StoredStageState::default()
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            path,
            state: RwLock::new(state),
            persistence: Mutex::new(()),
        })
    }

    async fn snapshot(&self) -> StoredStageState {
        self.state.read().await.clone()
    }

    async fn session_and_hold(&self) -> (String, bool) {
        let state = self.state.read().await;
        (state.session_id.clone(), state.held)
    }

    async fn mode(&self) -> WorkflowMode {
        self.state.read().await.workflow_mode
    }

    async fn agent_mode(&self) -> AgentMode {
        self.state.read().await.agent_mode
    }

    async fn session_id(&self) -> String {
        self.state.read().await.session_id.clone()
    }

    async fn set_mode(&self, mode: WorkflowMode) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        self.state.write().await.workflow_mode = mode;
        self.persist_locked().await
    }

    async fn set_agent_mode(&self, agent_mode: AgentMode) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        self.state.write().await.agent_mode = agent_mode;
        self.persist_locked().await
    }

    async fn insert_if_absent(
        &self,
        submission: StageSubmission,
        maximum: usize,
    ) -> Result<(StageSubmission, bool), InsertError> {
        let _guard = self.persistence.lock().await;
        {
            let mut state = self.state.write().await;
            if let Some(existing) = state
                .submissions
                .iter()
                .find(|existing| existing.id == submission.id)
            {
                return Ok((existing.clone(), false));
            }
            if state.submissions.len() >= maximum {
                return Err(InsertError::AtCapacity);
            }
            state.submissions.push(submission.clone());
        }
        self.persist_locked()
            .await
            .map_err(InsertError::Persistence)?;
        Ok((submission, true))
    }

    async fn mark_failed(&self, id: &str, error: String) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        if let Some(submission) = self
            .state
            .write()
            .await
            .submissions
            .iter_mut()
            .find(|submission| submission.id == id)
        {
            submission.status = SubmissionStatus::Failed;
            submission.error = Some(error);
        }
        self.persist_locked().await
    }

    async fn apply_update(
        &self,
        update: StageUpdate,
        replace_with_safe_display: bool,
    ) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        let mut state = self.state.write().await;
        if update.session_id != state.session_id {
            return Ok(());
        }
        if let Some(submission) = state
            .submissions
            .iter_mut()
            .find(|submission| submission.id == update.id)
        {
            submission.status = update.status;
            submission.error = update.error;
            // Replace the trace only with the latest non-empty list, so the plain
            // (empty) reports of the delivery tail never clear a built-up trace.
            if !update.agent_steps.is_empty() {
                submission.agent_steps = update.agent_steps;
            }
            if let Some(judgment) = update.judgment {
                if replace_with_safe_display {
                    submission.text = judgment.display_confession;
                }
                submission.category = Some(judgment.category);
                submission.judgment = Some(judgment.judgment);
                submission.severity = Some(if judgment.severity_reason.trim().is_empty() {
                    // Older judgments (replayed before severity_reason existed) have
                    // no reason; render the bare level without a dangling dash.
                    format!("Ferris Level {}/5", judgment.severity)
                } else {
                    format!(
                        "Ferris Level {}/5 — {}",
                        judgment.severity, judgment.severity_reason
                    )
                });
                submission.prescription = Some(format!(
                    "{} Suggested tools: {}.",
                    judgment.prescription,
                    judgment.suggested_tools.join(", ")
                ));
                submission.sentence = Some(judgment.sentence);
                submission.penance_reps = Some(judgment.severity);
                submission.penance = Some(judgment.penance);
                submission.penance_line = Some(judgment.penance_line);
                submission.award_scores = Some(judgment.award_scores);
            }
        }
        drop(state);
        self.persist_locked().await
    }

    async fn set_held(&self, held: bool) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        self.state.write().await.held = held;
        self.persist_locked().await
    }

    async fn releasable_workflows(&self) -> Vec<(String, String)> {
        let state = self.state.read().await;
        state
            .submissions
            .iter()
            .filter(|submission| {
                submission.session_id == state.session_id
                    && !matches!(
                        submission.status,
                        SubmissionStatus::Sent | SubmissionStatus::Failed
                    )
            })
            .map(|submission| (submission.id.clone(), submission.workflow_id.clone()))
            .collect()
    }

    async fn reset(&self) -> anyhow::Result<()> {
        let _guard = self.persistence.lock().await;
        *self.state.write().await = StoredStageState::default();
        self.persist_locked().await
    }

    async fn persist_locked(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let bytes = serde_json::to_vec_pretty(&*self.state.read().await)?;
        let temporary = temporary_path(&self.path);
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(temporary, &self.path).await?;
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", Ulid::new()));
    PathBuf::from(temporary)
}

fn awards_for(submissions: &[StageSubmission]) -> Awards {
    fn winner(
        submissions: &[StageSubmission],
        score: impl Fn(&crate::domain::AwardScores) -> u8,
    ) -> Option<String> {
        submissions
            .iter()
            .filter(|submission| submission.status == SubmissionStatus::Sent)
            .filter_map(|submission| Some((submission, score(submission.award_scores.as_ref()?))))
            .max_by_key(|(_, score)| *score)
            .map(|(submission, _)| submission.id.clone())
    }

    Awards {
        most_cursed: winner(submissions, |scores| scores.most_cursed),
        most_relatable: winner(submissions, |scores| scores.most_relatable),
        most_needlessly_rewritten: winner(submissions, |scores| scores.most_needlessly_rewritten),
    }
}

#[derive(Debug)]
struct ApiError(StatusCode, String);

#[derive(Debug)]
enum InsertError {
    AtCapacity,
    Persistence(anyhow::Error),
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, message.into())
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, message.into())
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, message.into())
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, message.into())
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self(StatusCode::CONFLICT, message.into())
    }

    fn too_many_requests(message: impl Into<String>) -> Self {
        Self(StatusCode::TOO_MANY_REQUESTS, message.into())
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        warn!(%error, "stage request failed");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal stage error".to_owned(),
        )
    }
}

fn optional_idempotency_key(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    headers
        .get("idempotency-key")
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| ApiError::bad_request("Idempotency-Key must be valid ASCII"))
        })
        .transpose()
}

fn validate_submission_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ApiError::bad_request(
            "submission identity must be 1-128 ASCII letters, digits, '-' or '_'",
        ));
    }
    Ok(())
}

fn required_form_field<'a>(
    fields: &'a [(String, String)],
    name: &str,
) -> Result<&'a str, ApiError> {
    form_field(fields, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("missing Twilio field {name}")))
}

fn empty_twiml() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        "<Response></Response>",
    )
        .into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{TemporalConfig, TwilioInboundConfig},
        domain::{AwardScores, Category},
        twilio::twilio_signature,
    };
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn awards_choose_highest_scored_submission() {
        let input = SubmissionInput {
            id: "first".into(),
            session_id: "show".into(),
            text: "one".into(),
            created_at: Utc::now(),
            hold_before_reply: true,
            agent_mode: AgentMode::default(),
        };
        let mut first = StageSubmission::received(&input, "wf-first".into());
        first.status = SubmissionStatus::Sent;
        first.category = Some(Category::Other);
        first.award_scores = Some(AwardScores {
            most_cursed: 90,
            most_relatable: 20,
            most_needlessly_rewritten: 10,
        });
        let mut second = first.clone();
        second.id = "second".into();
        second.award_scores.as_mut().unwrap().most_cursed = 10;
        let awards = awards_for(&[first, second]);
        assert_eq!(awards.most_cursed.as_deref(), Some("first"));
    }

    #[tokio::test]
    async fn insertion_is_idempotent_and_capacity_is_atomic() {
        let path = std::env::temp_dir().join(format!("rust-confessional-{}.json", Ulid::new()));
        let store = Arc::new(StageStore::load(path.clone()).await.unwrap());
        let input = SubmissionInput {
            id: "same-id".into(),
            session_id: "show".into(),
            text: "one".into(),
            created_at: Utc::now(),
            hold_before_reply: true,
            agent_mode: AgentMode::default(),
        };
        let first = StageSubmission::received(&input, "wf-first".into());
        let (stored, inserted) = store.insert_if_absent(first.clone(), 1).await.unwrap();
        assert!(inserted);
        assert_eq!(stored.id, first.id);

        let (stored, inserted) = store.insert_if_absent(first, 1).await.unwrap();
        assert!(!inserted);
        assert_eq!(stored.id, "same-id");

        let mut other_input = input;
        other_input.id = "other-id".into();
        let other = StageSubmission::received(&other_input, "wf-other".into());
        assert!(matches!(
            store.insert_if_absent(other, 1).await,
            Err(InsertError::AtCapacity)
        ));
        assert_eq!(store.snapshot().await.submissions.len(), 1);

        let _ = tokio::fs::remove_file(path).await;
    }

    #[test]
    fn submission_ids_preserve_case_and_reject_unsafe_characters() {
        validate_submission_id("twilio-SMabc123").unwrap();
        assert!(validate_submission_id("ABC").is_ok());
        assert!(validate_submission_id("abc").is_ok());
        assert!(validate_submission_id("contains/slash").is_err());
        assert!(validate_submission_id("").is_err());
    }

    #[test]
    fn submission_ids_enforce_the_length_and_charset_boundary() {
        // Dash and underscore are the only non-alphanumeric characters allowed;
        // the length cap is an inclusive 128 bytes.
        validate_submission_id("web-01ABC_def").unwrap();
        validate_submission_id(&"a".repeat(128)).unwrap();
        assert!(validate_submission_id(&"a".repeat(129)).is_err());
        assert!(validate_submission_id("has space").is_err());
        assert!(validate_submission_id("emoji-\u{1f600}").is_err());
    }

    #[tokio::test]
    async fn signed_twilio_compliance_message_is_acknowledged_without_submission() {
        let path = std::env::temp_dir().join(format!("rust-confessional-{}.json", Ulid::new()));
        let public_url = "https://demo.example/webhooks/twilio/messages";
        let auth_token = "test-token";
        let body =
            "AccountSid=AC123&MessageSid=SM123&From=%2B14155550100&To=%2B18005550100&Body=STOP";
        let fields = parse_form_body(body.as_bytes());
        let signature = twilio_signature(auth_token, public_url, &fields);
        let app = StageApp::new(StageConfig {
            bind_address: "127.0.0.1:0".parse().unwrap(),
            data_path: path.clone(),
            static_dir: "static".into(),
            internal_token: "internal".into(),
            max_confession_chars: 500,
            max_submissions_per_session: 20,
            show_raw_confessions: false,
            twilio: Some(TwilioInboundConfig {
                account_sid: "AC123".into(),
                auth_token: auth_token.into(),
                webhook_url: public_url.into(),
            }),
            twilio_poll: None,
            temporal: TemporalConfig {
                task_queue: "test".into(),
            },
            mask_words: Vec::new(),
        })
        .await
        .unwrap();
        let store = app.store.clone();
        let response = app
            .router()
            .oneshot(
                Request::post("/webhooks/twilio/messages")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header("x-twilio-signature", signature)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(store.snapshot().await.submissions.is_empty());
        let _ = tokio::fs::remove_file(path).await;
    }
}

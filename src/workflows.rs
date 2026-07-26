use std::time::Duration;

use temporalio_common::protos::temporal::api::common::v1::RetryPolicy;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, SyncWorkflowContext, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::{
    activities::{
        ComposeInput, ConfessionalActivities, DecideInput, DeliveryInput, LookupInput, SkillInput,
        stage_update,
    },
    domain::{
        AgentMode, AgentPlan, AgentStep, Category, Finding, Judgment, ReleaseInput, Remedy,
        SessionConfession, SessionSnapshot, Skill, SubmissionInput, SubmissionStatus,
        WorkflowSnapshot,
    },
};

/// The autonomous loop's hard step cap: a deterministic backstop so the loop
/// always terminates even if a backend never returns `Finish`.
const MAX_AGENT_STEPS: u8 = 6;

#[workflow]
pub struct ConfessionWorkflow {
    submission: SubmissionInput,
    status: SubmissionStatus,
    plan: Option<AgentPlan>,
    judgment: Option<Judgment>,
    released: bool,
    findings: Vec<Finding>,
    steps: Vec<AgentStep>,
}

#[workflow_methods]
impl ConfessionWorkflow {
    #[init]
    pub fn new(_ctx: &WorkflowContextView, submission: SubmissionInput) -> Self {
        Self {
            released: !submission.hold_before_reply,
            submission,
            status: SubmissionStatus::Received,
            plan: None,
            judgment: None,
            findings: Vec::new(),
            steps: Vec::new(),
        }
    }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<Judgment> {
        let submission = ctx.state(|state| state.submission.clone());

        // Both agent shapes must end by producing a Judgment; they then converge
        // on the same ReplyPending checkpoint and delivery tail below.
        let judgment = match submission.agent_mode {
            AgentMode::Linear => run_linear(ctx, &submission).await?,
            AgentMode::Autonomous => run_autonomous(ctx, &submission).await?,
        };

        ctx.state_mut(|state| {
            state.status = SubmissionStatus::ReplyPending;
            state.judgment = Some(judgment.clone());
        });
        report(
            ctx,
            &submission,
            SubmissionStatus::ReplyPending,
            Some(judgment.clone()),
        )
        .await;

        // This controlled checkpoint makes the stage failure deterministic. The Signal is
        // durable, so it is safe to arrive while this Worker is offline.
        ctx.wait_condition(|state| state.released).await;

        set_status(ctx, SubmissionStatus::Sending);
        report(
            ctx,
            &submission,
            SubmissionStatus::Sending,
            Some(judgment.clone()),
        )
        .await;
        let delivery_result = ctx
            .start_activity(
                ConfessionalActivities::deliver,
                DeliveryInput {
                    submission_id: submission.id.clone(),
                },
                // Delivery is at-least-once; deduplicate by submission id before raising attempts.
                activity_options(10, 30, 1),
            )
            .await;
        if let Err(error) = delivery_result {
            report_failure(ctx, &submission).await;
            return Err(error.into());
        }

        set_status(ctx, SubmissionStatus::Sent);
        report(
            ctx,
            &submission,
            SubmissionStatus::Sent,
            Some(judgment.clone()),
        )
        .await;
        Ok(judgment)
    }

    #[signal]
    pub fn release(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _input: ReleaseInput) {
        self.released = true;
    }

    #[query]
    pub fn snapshot(&self, _ctx: &WorkflowContextView) -> WorkflowSnapshot {
        WorkflowSnapshot {
            submission: self.submission.clone(),
            status: self.status,
            plan: self.plan.clone(),
            judgment: self.judgment.clone(),
            released: self.released,
            findings: self.findings.clone(),
            steps: self.steps.clone(),
        }
    }
}

/// The original fixed pipeline: plan, optionally look up a remedy, then compose.
/// Returns the composed Judgment; the shared tail owns the checkpoint/delivery.
async fn run_linear(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
) -> WorkflowResult<Judgment> {
    set_status(ctx, SubmissionStatus::Judging);
    report(ctx, submission, SubmissionStatus::Judging, None).await;
    let plan = match ctx
        .start_activity(
            ConfessionalActivities::plan,
            submission.clone(),
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            report_failure(ctx, submission).await;
            return Err(error.into());
        }
    };
    ctx.state_mut(|state| state.plan = Some(plan.clone()));

    let remedy = if plan.needs_lookup {
        set_status(ctx, SubmissionStatus::Researching);
        report(ctx, submission, SubmissionStatus::Researching, None).await;
        match ctx
            .start_activity(
                ConfessionalActivities::lookup_remedy,
                LookupInput { plan: plan.clone() },
                activity_options(5, 15, 3),
            )
            .await
        {
            Ok(remedy) => Some(remedy),
            Err(error) => {
                report_failure(ctx, submission).await;
                return Err(error.into());
            }
        }
    } else {
        None
    };

    set_status(ctx, SubmissionStatus::Composing);
    report(ctx, submission, SubmissionStatus::Composing, None).await;
    match ctx
        .start_activity(
            ConfessionalActivities::compose,
            ComposeInput {
                submission: submission.clone(),
                plan,
                remedy,
            },
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(judgment) => Ok(judgment),
        Err(error) => {
            report_failure(ctx, submission).await;
            Err(error.into())
        }
    }
}

/// The autonomous shape: plan once for a category, then loop up to the cap,
/// letting the backend decide each step (research / compose / revise / finish).
/// Guarantees a Judgment exists on return so the shared tail always has a draft.
async fn run_autonomous(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
) -> WorkflowResult<Judgment> {
    set_status(ctx, SubmissionStatus::Judging);
    report(ctx, submission, SubmissionStatus::Judging, None).await;
    let plan = match ctx
        .start_activity(
            ConfessionalActivities::plan,
            submission.clone(),
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            report_failure(ctx, submission).await;
            return Err(error.into());
        }
    };
    ctx.state_mut(|state| state.plan = Some(plan.clone()));

    for iteration in 0..MAX_AGENT_STEPS {
        let (findings, has_draft) =
            ctx.state(|state| (state.findings.clone(), state.judgment.is_some()));
        let step = match ctx
            .start_activity(
                ConfessionalActivities::decide_next_step,
                DecideInput {
                    text: submission.text.clone(),
                    category: plan.category,
                    findings: findings.clone(),
                    has_draft,
                    iteration,
                },
                activity_options(20, 75, 3),
            )
            .await
        {
            Ok(step) => step,
            Err(error) => {
                report_failure(ctx, submission).await;
                return Err(error.into());
            }
        };
        ctx.state_mut(|state| state.steps.push(step.clone()));

        match step {
            AgentStep::Lookup { skill, .. } => {
                set_status(ctx, SubmissionStatus::Researching);
                report(ctx, submission, SubmissionStatus::Researching, None).await;
                let finding = match ctx
                    .start_activity(
                        ConfessionalActivities::run_skill,
                        SkillInput {
                            skill,
                            text: submission.text.clone(),
                            category: plan.category,
                            findings,
                            has_draft,
                            iteration,
                        },
                        activity_options(20, 75, 3),
                    )
                    .await
                {
                    Ok(finding) => finding,
                    Err(error) => {
                        report_failure(ctx, submission).await;
                        return Err(error.into());
                    }
                };
                ctx.state_mut(|state| state.findings.push(finding));
            }
            AgentStep::Compose | AgentStep::Revise { .. } => {
                let remedy = ctx.state(|state| first_remedy(&state.findings, plan.category));
                let judgment = compose_draft(ctx, submission, &plan, remedy).await?;
                ctx.state_mut(|state| state.judgment = Some(judgment));
            }
            AgentStep::Finish => break,
        }
    }

    // The loop can finish before ever composing (a backend that finishes early);
    // guarantee a Judgment so the shared checkpoint/delivery tail has a draft.
    if let Some(judgment) = ctx.state(|state| state.judgment.clone()) {
        return Ok(judgment);
    }
    let remedy = ctx.state(|state| first_remedy(&state.findings, plan.category));
    let judgment = compose_draft(ctx, submission, &plan, remedy).await?;
    ctx.state_mut(|state| state.judgment = Some(judgment.clone()));
    Ok(judgment)
}

/// Set Composing, report, and run the `compose` Activity, mapping failure onto
/// the shared failure path. Shared by every compose/revise step of the loop.
async fn compose_draft(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
    plan: &AgentPlan,
    remedy: Option<Remedy>,
) -> WorkflowResult<Judgment> {
    set_status(ctx, SubmissionStatus::Composing);
    report(ctx, submission, SubmissionStatus::Composing, None).await;
    match ctx
        .start_activity(
            ConfessionalActivities::compose,
            ComposeInput {
                submission: submission.clone(),
                plan: plan.clone(),
                remedy,
            },
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(judgment) => Ok(judgment),
        Err(error) => {
            report_failure(ctx, submission).await;
            Err(error.into())
        }
    }
}

/// Rebuild the approved `Remedy` from the first `RemedyLookup` finding, if the
/// loop gathered one; otherwise the compose step runs without an approved remedy.
fn first_remedy(findings: &[Finding], category: Category) -> Option<Remedy> {
    findings
        .iter()
        .find(|finding| finding.skill == Skill::RemedyLookup)
        .map(|finding| Remedy {
            category,
            guidance: finding.summary.clone(),
            suggested_tools: finding
                .detail
                .split(", ")
                .filter(|tool| !tool.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
        })
}

/// The aggregate demo variant: a single long-lived Workflow for an entire
/// session. Confessions arrive by Signal and are folded into one durable state
/// via `state_mut`. Same Activities as the per-confession Workflow, different
/// granularity. In production you would run one Workflow per confession (see
/// `ConfessionWorkflow`); this exists to make durable state visible on stage.
#[workflow]
pub struct SessionWorkflow {
    session_id: String,
    confessions: Vec<SessionConfession>,
}

#[workflow_methods]
impl SessionWorkflow {
    #[init]
    pub fn new(_ctx: &WorkflowContextView, session_id: String) -> Self {
        Self {
            session_id,
            confessions: Vec::new(),
        }
    }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<()> {
        // Two phases, one item at a time so the aggregate stays deterministic and
        // clear of the preview SDK's dynamic-future edges: compose every received
        // confession first (the board fills with held judgments), then deliver the
        // ones that have been released. A failed item is isolated and never fails
        // the whole session.
        loop {
            ctx.wait_condition(|state| {
                state.confessions.iter().any(is_composable)
                    || state.confessions.iter().any(is_deliverable)
            })
            .await;

            if let Some(id) = ctx.state(|state| first_matching(state, is_composable)) {
                compose_confession(ctx, id).await;
            } else if let Some(id) = ctx.state(|state| first_matching(state, is_deliverable)) {
                deliver_confession(ctx, id).await;
            }
        }
    }

    #[signal]
    pub fn add_confession(
        &mut self,
        _ctx: &mut SyncWorkflowContext<Self>,
        submission: SubmissionInput,
    ) {
        if self
            .confessions
            .iter()
            .any(|item| item.submission.id == submission.id)
        {
            return;
        }
        // Honor the per-confession hold, exactly like ConfessionWorkflow::new.
        let released = !submission.hold_before_reply;
        self.confessions.push(SessionConfession {
            submission,
            status: SubmissionStatus::Received,
            plan: None,
            judgment: None,
            released,
        });
    }

    #[signal]
    pub fn release(&mut self, _ctx: &mut SyncWorkflowContext<Self>, _input: ReleaseInput) {
        // Free every confession that has not already finished.
        for item in &mut self.confessions {
            if !matches!(
                item.status,
                SubmissionStatus::Sent | SubmissionStatus::Failed
            ) {
                item.released = true;
            }
        }
    }

    #[query]
    pub fn snapshot(&self, _ctx: &WorkflowContextView) -> SessionSnapshot {
        SessionSnapshot {
            session_id: self.session_id.clone(),
            confessions: self.confessions.clone(),
        }
    }
}

fn is_composable(item: &SessionConfession) -> bool {
    matches!(item.status, SubmissionStatus::Received)
}

fn is_deliverable(item: &SessionConfession) -> bool {
    matches!(item.status, SubmissionStatus::ReplyPending) && item.released
}

fn first_matching(
    state: &SessionWorkflow,
    predicate: fn(&SessionConfession) -> bool,
) -> Option<String> {
    state
        .confessions
        .iter()
        .find(|&item| predicate(item))
        .map(|item| item.submission.id.clone())
}

fn find_submission(
    ctx: &mut WorkflowContext<SessionWorkflow>,
    id: &str,
) -> Option<SubmissionInput> {
    ctx.state(|state| {
        state
            .confessions
            .iter()
            .find(|item| item.submission.id == id)
            .map(|item| item.submission.clone())
    })
}

/// Judge, research, and compose one confession, parking it at `ReplyPending`.
/// Delivery is intentionally not gated here so the whole board can fill with held
/// judgments before any release.
async fn compose_confession(ctx: &mut WorkflowContext<SessionWorkflow>, id: String) {
    let Some(submission) = find_submission(ctx, &id) else {
        return;
    };
    set_item(ctx, &submission, SubmissionStatus::Judging, None).await;
    let plan = match ctx
        .start_activity(
            ConfessionalActivities::plan,
            submission.clone(),
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(plan) => plan,
        Err(_) => return fail_item(ctx, &submission).await,
    };
    update_item(ctx, &id, |item| item.plan = Some(plan.clone()));

    let remedy = if plan.needs_lookup {
        set_item(ctx, &submission, SubmissionStatus::Researching, None).await;
        match ctx
            .start_activity(
                ConfessionalActivities::lookup_remedy,
                LookupInput { plan: plan.clone() },
                activity_options(5, 15, 3),
            )
            .await
        {
            Ok(remedy) => Some(remedy),
            Err(_) => return fail_item(ctx, &submission).await,
        }
    } else {
        None
    };

    set_item(ctx, &submission, SubmissionStatus::Composing, None).await;
    let judgment = match ctx
        .start_activity(
            ConfessionalActivities::compose,
            ComposeInput {
                submission: submission.clone(),
                plan,
                remedy,
            },
            activity_options(20, 75, 3),
        )
        .await
    {
        Ok(judgment) => judgment,
        Err(_) => return fail_item(ctx, &submission).await,
    };
    update_item(ctx, &id, |item| {
        item.status = SubmissionStatus::ReplyPending;
        item.judgment = Some(judgment.clone());
    });
    report_session(
        ctx,
        &submission,
        SubmissionStatus::ReplyPending,
        Some(judgment),
    )
    .await;
}

/// Deliver one released confession, moving it from `ReplyPending` to `Sent`.
async fn deliver_confession(ctx: &mut WorkflowContext<SessionWorkflow>, id: String) {
    let Some(submission) = find_submission(ctx, &id) else {
        return;
    };
    let judgment = ctx.state(|state| {
        state
            .confessions
            .iter()
            .find(|item| item.submission.id == id)
            .and_then(|item| item.judgment.clone())
    });

    set_item(
        ctx,
        &submission,
        SubmissionStatus::Sending,
        judgment.clone(),
    )
    .await;
    let delivery_result = ctx
        .start_activity(
            ConfessionalActivities::deliver,
            DeliveryInput {
                submission_id: submission.id.clone(),
            },
            activity_options(10, 30, 1),
        )
        .await;
    if delivery_result.is_err() {
        return fail_item(ctx, &submission).await;
    }
    set_item(ctx, &submission, SubmissionStatus::Sent, judgment).await;
}

fn update_item<F>(ctx: &mut WorkflowContext<SessionWorkflow>, id: &str, mutate: F)
where
    F: FnOnce(&mut SessionConfession),
{
    ctx.state_mut(|state| {
        if let Some(item) = state
            .confessions
            .iter_mut()
            .find(|item| item.submission.id == id)
        {
            mutate(item);
        }
    });
}

async fn set_item(
    ctx: &mut WorkflowContext<SessionWorkflow>,
    submission: &SubmissionInput,
    status: SubmissionStatus,
    judgment: Option<Judgment>,
) {
    update_item(ctx, &submission.id, |item| item.status = status);
    report_session(ctx, submission, status, judgment).await;
}

async fn report_session(
    ctx: &mut WorkflowContext<SessionWorkflow>,
    submission: &SubmissionInput,
    status: SubmissionStatus,
    judgment: Option<Judgment>,
) {
    let _ = ctx
        .start_activity(
            ConfessionalActivities::report_stage,
            stage_update(submission, status, judgment),
            activity_options(5, 12, 2),
        )
        .await;
}

async fn fail_item(ctx: &mut WorkflowContext<SessionWorkflow>, submission: &SubmissionInput) {
    update_item(ctx, &submission.id, |item| {
        item.status = SubmissionStatus::Failed
    });
    let _ = ctx
        .start_activity(
            ConfessionalActivities::report_stage,
            failure_update(submission),
            activity_options(5, 12, 2),
        )
        .await;
}

fn failure_update(submission: &SubmissionInput) -> crate::domain::StageUpdate {
    crate::domain::StageUpdate {
        id: submission.id.clone(),
        session_id: submission.session_id.clone(),
        status: SubmissionStatus::Failed,
        judgment: None,
        error: Some("Agent step failed; inspect the Worker logs for details.".to_owned()),
    }
}

fn set_status(ctx: &mut WorkflowContext<ConfessionWorkflow>, status: SubmissionStatus) {
    ctx.state_mut(|state| state.status = status);
}

async fn report(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
    status: SubmissionStatus,
    judgment: Option<Judgment>,
) {
    // The dashboard is a projection. Its failure must never fail the durable agent.
    let _ = ctx
        .start_activity(
            ConfessionalActivities::report_stage,
            stage_update(submission, status, judgment),
            activity_options(5, 12, 2),
        )
        .await;
}

async fn report_failure(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
) {
    set_status(ctx, SubmissionStatus::Failed);
    let _ = ctx
        .start_activity(
            ConfessionalActivities::report_stage,
            failure_update(submission),
            activity_options(5, 12, 2),
        )
        .await;
}

fn activity_options(
    start_to_close: u64,
    schedule_to_close: u64,
    maximum_attempts: i32,
) -> ActivityOptions {
    ActivityOptions::with_close_timeouts(ActivityCloseTimeouts::Both {
        start_to_close: Duration::from_secs(start_to_close),
        schedule_to_close: Duration::from_secs(schedule_to_close),
    })
    .retry_policy(RetryPolicy {
        maximum_attempts,
        ..Default::default()
    })
    .build()
}

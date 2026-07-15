use std::time::Duration;

use temporalio_common::protos::temporal::api::common::v1::RetryPolicy;
use temporalio_macros::{workflow, workflow_methods};
use temporalio_sdk::{
    ActivityCloseTimeouts, ActivityOptions, SyncWorkflowContext, WorkflowContext,
    WorkflowContextView, WorkflowResult,
};

use crate::{
    activities::{ComposeInput, ConfessionalActivities, DeliveryInput, LookupInput, stage_update},
    domain::{
        AgentPlan, Judgment, ReleaseInput, SubmissionInput, SubmissionStatus, WorkflowSnapshot,
    },
};

#[workflow]
pub struct ConfessionWorkflow {
    submission: SubmissionInput,
    status: SubmissionStatus,
    plan: Option<AgentPlan>,
    judgment: Option<Judgment>,
    released: bool,
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
        }
    }

    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>) -> WorkflowResult<Judgment> {
        let submission = ctx.state(|state| state.submission.clone());

        set_status(ctx, SubmissionStatus::Judging);
        report(ctx, &submission, SubmissionStatus::Judging, None).await;
        let plan_result = ctx
            .start_activity(
                ConfessionalActivities::plan,
                submission.clone(),
                model_activity_options(),
            )
            .await;
        let plan = match plan_result {
            Ok(plan) => plan,
            Err(error) => {
                report_failure(ctx, &submission).await;
                return Err(error.into());
            }
        };
        ctx.state_mut(|state| state.plan = Some(plan.clone()));

        let remedy = if plan.needs_lookup {
            set_status(ctx, SubmissionStatus::Researching);
            report(ctx, &submission, SubmissionStatus::Researching, None).await;
            let lookup_result = ctx
                .start_activity(
                    ConfessionalActivities::lookup_remedy,
                    LookupInput { plan: plan.clone() },
                    tool_activity_options(),
                )
                .await;
            match lookup_result {
                Ok(remedy) => Some(remedy),
                Err(error) => {
                    report_failure(ctx, &submission).await;
                    return Err(error.into());
                }
            }
        } else {
            None
        };

        set_status(ctx, SubmissionStatus::Composing);
        report(ctx, &submission, SubmissionStatus::Composing, None).await;
        let judgment_result = ctx
            .start_activity(
                ConfessionalActivities::compose,
                ComposeInput {
                    submission: submission.clone(),
                    plan,
                    remedy,
                },
                model_activity_options(),
            )
            .await;
        let judgment = match judgment_result {
            Ok(judgment) => judgment,
            Err(error) => {
                report_failure(ctx, &submission).await;
                return Err(error.into());
            }
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
                delivery_activity_options(),
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
        }
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
            report_activity_options(),
        )
        .await;
}

async fn report_failure(
    ctx: &mut WorkflowContext<ConfessionWorkflow>,
    submission: &SubmissionInput,
) {
    set_status(ctx, SubmissionStatus::Failed);
    let update = crate::domain::StageUpdate {
        id: submission.id.clone(),
        session_id: submission.session_id.clone(),
        status: SubmissionStatus::Failed,
        judgment: None,
        error: Some("Agent step failed; inspect the Worker logs for details.".to_owned()),
    };
    let _ = ctx
        .start_activity(
            ConfessionalActivities::report_stage,
            update,
            report_activity_options(),
        )
        .await;
}

fn retry_policy(maximum_attempts: i32) -> RetryPolicy {
    RetryPolicy {
        maximum_attempts,
        ..Default::default()
    }
}

fn model_activity_options() -> ActivityOptions {
    ActivityOptions::with_close_timeouts(ActivityCloseTimeouts::Both {
        start_to_close: Duration::from_secs(20),
        schedule_to_close: Duration::from_secs(75),
    })
    .retry_policy(retry_policy(3))
    .build()
}

fn tool_activity_options() -> ActivityOptions {
    ActivityOptions::with_close_timeouts(ActivityCloseTimeouts::Both {
        start_to_close: Duration::from_secs(5),
        schedule_to_close: Duration::from_secs(15),
    })
    .retry_policy(retry_policy(3))
    .build()
}

fn report_activity_options() -> ActivityOptions {
    ActivityOptions::with_close_timeouts(ActivityCloseTimeouts::Both {
        start_to_close: Duration::from_secs(5),
        schedule_to_close: Duration::from_secs(12),
    })
    .retry_policy(retry_policy(2))
    .build()
}

fn delivery_activity_options() -> ActivityOptions {
    ActivityOptions::with_close_timeouts(ActivityCloseTimeouts::Both {
        start_to_close: Duration::from_secs(10),
        schedule_to_close: Duration::from_secs(30),
    })
    // Delivery implementations must deduplicate by submission id before increasing this.
    .retry_policy(retry_policy(1))
    .build()
}

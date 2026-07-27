use temporalio_client::{
    Client, ClientOptions, Connection, WorkflowSignalOptions, WorkflowStartOptions,
    envconfig::LoadClientConfigProfileOptions,
};
use temporalio_common::protos::temporal::api::enums::v1::{
    WorkflowIdConflictPolicy, WorkflowIdReusePolicy,
};

use crate::{
    WORKFLOW_ID_PREFIX,
    domain::{ReleaseInput, SubmissionInput},
    workflows::{ConfessionWorkflow, SessionWorkflow},
};

pub async fn connect_client() -> anyhow::Result<Client> {
    let (connection_options, client_options) =
        ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let connection = Connection::connect(connection_options).await?;
    Ok(Client::new(connection, client_options)?)
}

pub fn workflow_id(submission_id: &str) -> String {
    format!("{WORKFLOW_ID_PREFIX}-{submission_id}")
}

pub async fn start_submission(
    client: &Client,
    task_queue: &str,
    submission: SubmissionInput,
) -> anyhow::Result<String> {
    let workflow_id = workflow_id(&submission.id);
    let options = WorkflowStartOptions::new(task_queue, &workflow_id)
        .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
        .build();
    let start_result = client
        .start_workflow(ConfessionWorkflow::run, submission, options)
        .await;

    match start_result {
        Ok(_) => Ok(workflow_id),
        Err(start_error) => {
            // A source retry can arrive after the Workflow has already completed. Confirm
            // that the stable Workflow ID exists before treating the failed start as success.
            // This also closes the Stage-store-before-Temporal-start crash window safely.
            let existing = client
                .get_workflow_handle::<ConfessionWorkflow>(&workflow_id)
                .describe(Default::default())
                .await;
            if existing.is_ok() {
                Ok(workflow_id)
            } else {
                Err(start_error.into())
            }
        }
    }
}

pub async fn release_submission(
    client: &Client,
    workflow_id: &str,
    request_id: &str,
) -> anyhow::Result<()> {
    let handle = client.get_workflow_handle::<ConfessionWorkflow>(workflow_id);
    let mut options = WorkflowSignalOptions::default();
    options.request_id = Some(request_id.to_owned());
    handle
        .signal(
            ConfessionWorkflow::release,
            ReleaseInput {
                reason: "stage operator released replies".to_owned(),
            },
            options,
        )
        .await?;
    Ok(())
}

pub fn session_workflow_id(session_id: &str) -> String {
    format!("{WORKFLOW_ID_PREFIX}-session-{session_id}")
}

/// Ensure the aggregate Workflow for this session exists. Idempotent: repeated
/// calls return the existing execution rather than starting a second one.
pub async fn start_session(
    client: &Client,
    task_queue: &str,
    session_id: &str,
) -> anyhow::Result<String> {
    let workflow_id = session_workflow_id(session_id);
    let options = WorkflowStartOptions::new(task_queue, &workflow_id)
        .id_conflict_policy(WorkflowIdConflictPolicy::UseExisting)
        .id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate)
        .build();
    let start_result = client
        .start_workflow(SessionWorkflow::run, session_id.to_owned(), options)
        .await;

    match start_result {
        Ok(_) => Ok(workflow_id),
        Err(start_error) => {
            let existing = client
                .get_workflow_handle::<SessionWorkflow>(&workflow_id)
                .describe(Default::default())
                .await;
            if existing.is_ok() {
                Ok(workflow_id)
            } else {
                Err(start_error.into())
            }
        }
    }
}

pub async fn add_session_confession(
    client: &Client,
    workflow_id: &str,
    submission: SubmissionInput,
    request_id: &str,
) -> anyhow::Result<()> {
    let handle = client.get_workflow_handle::<SessionWorkflow>(workflow_id);
    let mut options = WorkflowSignalOptions::default();
    options.request_id = Some(request_id.to_owned());
    handle
        .signal(SessionWorkflow::add_confession, submission, options)
        .await?;
    Ok(())
}

pub async fn release_session(
    client: &Client,
    workflow_id: &str,
    request_id: &str,
) -> anyhow::Result<()> {
    let handle = client.get_workflow_handle::<SessionWorkflow>(workflow_id);
    let mut options = WorkflowSignalOptions::default();
    options.request_id = Some(request_id.to_owned());
    handle
        .signal(
            SessionWorkflow::release,
            ReleaseInput {
                reason: "stage operator released replies".to_owned(),
            },
            options,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_ids_are_stable_and_stage_readable() {
        assert_eq!(workflow_id("01ABC"), "rust-confession-01ABC");
    }

    #[test]
    fn session_workflow_ids_are_stable_and_stage_readable() {
        assert_eq!(
            session_workflow_id("01SESS"),
            "rust-confession-session-01SESS"
        );
    }
}

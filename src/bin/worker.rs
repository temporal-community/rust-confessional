use std::time::Duration;

use rust_confessional::{
    activities::{ConfessionalActivities, StageReporter},
    agent::build_backend,
    config::WorkerConfig,
    init_tracing, temporal,
    workflows::ConfessionWorkflow,
};
use temporalio_sdk::{Worker, WorkerOptions};
use temporalio_sdk_core::{CoreRuntime, RuntimeOptions};
use tokio::time::sleep;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let config = WorkerConfig::from_env()?;
    let backend = build_backend(&config)?;
    let model_mode = backend.mode();
    let reporter = StageReporter::new(
        config.stage_internal_url.clone(),
        config.stage_internal_token.clone(),
    )?;
    tokio::spawn(reporter.heartbeat_loop(model_mode));

    let activities = ConfessionalActivities::new(
        backend,
        config.stage_internal_url,
        config.stage_internal_token,
    )?;
    let runtime_options = RuntimeOptions::builder()
        .build()
        .map_err(anyhow::Error::msg)?;
    let runtime = CoreRuntime::new_assume_tokio(runtime_options)?;
    let client = loop {
        match temporal::connect_client().await {
            Ok(client) => break client,
            Err(error) => {
                warn!(%error, "waiting for Temporal");
                sleep(Duration::from_secs(2)).await;
            }
        }
    };

    let options = WorkerOptions::new(&config.temporal.task_queue)
        .register_workflow::<ConfessionWorkflow>()?
        .register_activities(activities)
        .build();
    let mut worker = Worker::new(&runtime, client, options)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    info!(
        task_queue = %config.temporal.task_queue,
        model_mode,
        "durable agent Worker started"
    );
    worker.run().await?;
    Ok(())
}

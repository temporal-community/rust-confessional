pub mod activities;
pub mod agent;
pub mod config;
pub mod domain;
pub mod stage;
pub mod temporal;
pub(crate) mod twilio;
pub mod workflows;

pub const WORKFLOW_ID_PREFIX: &str = "rust-confession";

pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "rust_confessional=info".into());

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init();
}

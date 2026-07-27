use std::env;

use chrono::Utc;
use rust_confessional::{
    agent::{AgentBackend, FixtureBackend, remedy_for},
    domain::SubmissionInput,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let confession = env::args().skip(1).collect::<Vec<_>>().join(" ");
    if confession.trim().is_empty() {
        println!("Recovered pending confessions: 0");
        println!("Nothing to resume—the process memory is empty.");
        return Ok(());
    }

    let input = SubmissionInput {
        id: "naive-in-memory".to_owned(),
        session_id: "not-durable".to_owned(),
        text: confession,
        created_at: Utc::now(),
        hold_before_reply: true,
        agent_mode: Default::default(),
    };
    let backend = FixtureBackend;

    println!("RECEIVED       {}", input.text);
    println!("PLANNING       classify and choose a tool");
    let plan = backend.plan(&input).await?;
    println!(
        "TOOL           approved remedy catalog / {}",
        plan.search_key
    );
    let remedy = remedy_for(plan.category);
    println!("COMPOSING      Ferris is sharpening the judgment");
    let judgment = backend.compose(&input, &plan, Some(&remedy), &[]).await?;

    let pending_in_memory = [judgment];
    println!("REPLY PENDING  memory only — kill this container now");
    println!(
        "Pending confessions in this process: {}",
        pending_in_memory.len()
    );

    std::future::pending::<anyhow::Result<()>>().await
}

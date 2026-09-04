use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{info, error};

use crate::agent::{AgentLoopContext, run_agent_loop};
use crate::error::AlfredError;
use crate::server::AppState;

pub struct Scheduler {
    state: AppState,
}

impl Scheduler {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub async fn start(&self) -> Result<(), AlfredError> {
        info!("Starting scheduler...");
        let sched = JobScheduler::new().await
            .map_err(|e| AlfredError::Scheduler(e.to_string()))?;

        // Add a daily summary job at 9am
        let state = self.state.clone();
        let daily_job = Job::new("0 9 * * *", move |_, _| {
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(e) = run_scheduled_prompt(&state, "Generate a daily summary of all active todos and pending items. List them with priorities.").await {
                    error!("Daily summary failed: {}", e);
                }
            });
        }).map_err(|e| AlfredError::Scheduler(e.to_string()))?;

        sched.add(daily_job).await
            .map_err(|e| AlfredError::Scheduler(e.to_string()))?;

        info!("Scheduler started with daily summary job");
        sched.start().await
            .map_err(|e| AlfredError::Scheduler(e.to_string()))?;

        Ok(())
    }
}

async fn run_scheduled_prompt(state: &AppState, prompt: &str) -> Result<(), AlfredError> {
    let event_tx = state.event_tx.clone();
    let mut ctx = AgentLoopContext {
        system_prompt: state.system_prompt.clone(),
        messages: vec![crate::types::Message::User(crate::types::UserMessage {
            content: vec![crate::types::Content::Text(crate::types::TextContent { text: prompt.to_string() })],
            timestamp: chrono::Utc::now(),
        })],
        provider: state.provider.clone(),
        model: state.model.clone(),
        tools: state.tools.clone(),
        event_tx,
        max_turns: 5,
    };

    run_agent_loop(&mut ctx).await;
    info!("Scheduled prompt completed");
    Ok(())
}

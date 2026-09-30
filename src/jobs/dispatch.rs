//! The composed job dispatcher: run a due job, then deliver its result.
//!
//! [`JobDispatch`] is the `Dispatch` the server installs into the scheduler. It
//! composes the two halves built by earlier issues: [`JobRunner`] runs the job
//! through a one-shot Pi subprocess and reports a [`RunEnd`]; [`Delivery`] sends
//! that result to the user and records whether it arrived.
//!
//! # Why delivery is a separate hook
//!
//! [`crate::scheduler::Dispatch::dispatch`] runs *before* the scheduler closes
//! the run row, so at that point the run's id is unknown — and the id is exactly
//! what delivery records its outcome against. The scheduler therefore calls
//! [`crate::scheduler::Dispatch::after_run`] once the row is closed, handing
//! over that id; delivery belongs there.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{JobsConfig, PiConfig, TelegramConfig};
use crate::error::AlfredError;
use crate::jobs::delivery::Delivery;
use crate::jobs::runner::{JobRunner, MissingVerdict};
use crate::jobs::{Job, RunEnd};
use crate::scheduler::Dispatch;
use crate::store::Store;

/// Runs a due job through Pi and delivers its result.
pub struct JobDispatch {
    store: Arc<Store>,
    runner: Arc<JobRunner>,
    delivery: Arc<Delivery>,
}

impl JobDispatch {
    /// Compose a runner and a delivery over the same store.
    pub fn new(store: Arc<Store>, runner: Arc<JobRunner>, delivery: Arc<Delivery>) -> Self {
        Self {
            store,
            runner,
            delivery,
        }
    }

    /// Compose the dispatcher from the loaded `[pi]`, `[jobs]`, and `[telegram]`
    /// configuration.
    ///
    /// This is the single place the scheduler and the manual-run entry points
    /// build their dispatcher, so a manual run and a scheduled run always use
    /// the same runner, verdict fallback, token, and recipient fallback.
    pub fn from_config(
        store: Arc<Store>,
        pi: &PiConfig,
        jobs: &JobsConfig,
        telegram: Option<&TelegramConfig>,
    ) -> Self {
        let runner = Arc::new(JobRunner::new(
            pi.clone(),
            MissingVerdict::parse(&jobs.missing_verdict),
        ));
        let token = telegram.and_then(|telegram| telegram.bot_token.clone());
        let allowed_users: &[u64] = telegram
            .map(|telegram| telegram.allowed_users.as_slice())
            .unwrap_or(&[]);
        let delivery = Arc::new(match store.telegram_sender_override() {
            Some(sender) => Delivery::new(token, allowed_users).with_sender(sender),
            None => Delivery::new(token, allowed_users),
        });
        Self::new(store, runner, delivery)
    }
}

#[async_trait]
impl Dispatch for JobDispatch {
    async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError> {
        self.runner.run(job).await
    }

    async fn after_run(&self, run_id: &str, job: &Job, end: &RunEnd) {
        // `Delivery::deliver` applies the report policy, records `delivered` (and
        // any delivery error) on the run row, and never touches its status or
        // output. Its outcome is logged inside; shutdown and the scheduler do not
        // depend on it.
        self.delivery.deliver(&self.store, run_id, job, end).await;
    }
}

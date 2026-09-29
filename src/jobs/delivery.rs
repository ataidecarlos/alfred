//! Job result delivery to the user's Telegram chat (issue #13).
//!
//! A scheduled job can run, produce a result, and still reach nobody: the reply
//! is generated and then dropped. This module closes that hole. It decides
//! whether a run's result should be delivered (the job's `report` policy plus,
//! for `on_signal`, the run's verdict), resolves the destination chat, and sends
//! the run's output through the shared Telegram send path
//! ([`crate::connectors::telegram::send_message`]).
//!
//! # Recording the outcome
//!
//! Delivery happens after the run row is closed, so the outcome is written back
//! with [`crate::store::Store::record_delivery`]: `delivered = 1` on success, and
//! on failure `delivered = 0` plus the error. The run's `status` and `output`
//! are never touched — a delivery failure must not turn a successful run into a
//! failed one, and its output must not be discarded.
//!
//! # Configuration
//!
//! The bot token and the `[telegram].allowed_users` fallback are constructor
//! parameters, so the caller injects the loaded config and this module never
//! reads the config file.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{error, info, warn};

use crate::connectors::telegram::{ApiMessageSender, MessageSender};
use crate::error::AlfredError;
use crate::jobs::runner::VERDICT_MATCH;
use crate::jobs::{Job, ReportPolicy, RunEnd};
use crate::store::Store;

/// Why a run's result was not delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The report policy did not ask for this run (for example, `on_signal`
    /// with a `NO_MATCH` verdict).
    NotWanted,
    /// The job has no `deliver_to` and `[telegram].allowed_users` is empty.
    NoRecipient,
    /// The run produced no output to send (for example, a timeout).
    NoOutput,
}

/// What happened when a run's result was offered to the delivery path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryOutcome {
    /// The output was sent to `chat_id` and the run was marked delivered.
    Delivered { chat_id: i64 },
    /// Nothing was sent; `reason` says why. The run stays `delivered = 0`.
    Skipped { reason: SkipReason },
    /// Delivery was required but did not happen; the error is recorded on the
    /// run. The run's status and output are unchanged.
    Failed { chat_id: Option<i64>, error: String },
}

/// Sends a job's result to the configured Telegram chat.
///
/// The token and the `allowed_users` fallback are injected; the transport is a
/// [`MessageSender`] so tests record messages instead of reaching the network.
pub struct Delivery {
    allowed_users: Vec<u64>,
    sender: Arc<dyn MessageSender>,
}

impl Delivery {
    /// Build delivery from the `[telegram]` configuration.
    ///
    /// `token` is the Bot API token; `allowed_users` supplies the fallback chat
    /// when a job has no `deliver_to`. A missing token is not a silent skip: the
    /// send fails and the failure is recorded on the run.
    pub fn new(token: Option<String>, allowed_users: &[u64]) -> Self {
        let sender: Arc<dyn MessageSender> = match token {
            Some(token) => Arc::new(ApiMessageSender::new(token)),
            None => Arc::new(UnconfiguredSender),
        };
        Self {
            allowed_users: allowed_users.to_vec(),
            sender,
        }
    }

    /// Replace the outbound transport (tests inject a recorder, so no request
    /// leaves the process).
    pub fn with_sender(mut self, sender: Arc<dyn MessageSender>) -> Self {
        self.sender = sender;
        self
    }

    /// Whether `job`'s report policy asks for `end` to be delivered.
    ///
    /// `always` delivers every run; `on_signal` delivers only a `MATCH` verdict.
    /// A run with no verdict satisfies neither.
    pub fn wants_delivery(job: &Job, end: &RunEnd) -> bool {
        match job.report {
            ReportPolicy::Always => true,
            ReportPolicy::OnSignal => end.verdict.as_deref() == Some(VERDICT_MATCH),
        }
    }

    /// Resolve the chat a job's result goes to.
    ///
    /// The job's own `deliver_to` wins; otherwise the first configured allowed
    /// user is the fallback. `Err` carries a configuration error (an
    /// unparseable `deliver_to`) so the caller can record it rather than guess.
    fn resolve_chat(&self, job: &Job) -> Result<Option<i64>, String> {
        match job
            .deliver_to
            .as_deref()
            .map(str::trim)
            .filter(|target| !target.is_empty())
        {
            Some(target) => target
                .parse::<i64>()
                .map(Some)
                .map_err(|_| format!("job '{}' has an invalid deliver_to '{target}'", job.name)),
            None => Ok(self
                .allowed_users
                .first()
                .copied()
                .and_then(|id| i64::try_from(id).ok())),
        }
    }

    /// Deliver `end` for `job` and record the outcome on the run row `run_id`.
    ///
    /// A policy that does not want the run, a missing recipient, and an empty
    /// output all leave `delivered = 0` and are logged. A send failure is logged
    /// at error, recorded on the run, and leaves the run's status and output
    /// untouched. The returned outcome is for the caller's own logging/tests.
    pub async fn deliver(
        &self,
        store: &Store,
        run_id: &str,
        job: &Job,
        end: &RunEnd,
    ) -> DeliveryOutcome {
        if !Self::wants_delivery(job, end) {
            return DeliveryOutcome::Skipped {
                reason: SkipReason::NotWanted,
            };
        }

        let Some(text) = end
            .output
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            warn!(job = %job.name, run_id, "run produced no output to deliver");
            return DeliveryOutcome::Skipped {
                reason: SkipReason::NoOutput,
            };
        };

        let chat_id = match self.resolve_chat(job) {
            Ok(Some(chat_id)) => chat_id,
            Ok(None) => {
                warn!(
                    job = %job.name,
                    run_id,
                    "no deliver_to and no telegram.allowed_users fallback; skipping delivery"
                );
                return DeliveryOutcome::Skipped {
                    reason: SkipReason::NoRecipient,
                };
            }
            Err(message) => {
                error!(job = %job.name, run_id, %message, "delivery configuration is invalid");
                record(store, run_id, false, Some(&message));
                return DeliveryOutcome::Failed {
                    chat_id: None,
                    error: message,
                };
            }
        };

        match self.sender.send(chat_id, text).await {
            Ok(()) => {
                record(store, run_id, true, None);
                info!(job = %job.name, run_id, chat_id, "delivered job result");
                DeliveryOutcome::Delivered { chat_id }
            }
            Err(error) => {
                let message = format!("delivery failed: {error}");
                error!(
                    job = %job.name,
                    run_id,
                    chat_id,
                    %error,
                    "delivery failed; the run output is retained"
                );
                record(store, run_id, false, Some(&message));
                DeliveryOutcome::Failed {
                    chat_id: Some(chat_id),
                    error: message,
                }
            }
        }
    }
}

/// Record a delivery outcome, logging (never unwinding) a store failure.
fn record(store: &Store, run_id: &str, delivered: bool, error: Option<&str>) {
    if let Err(store_error) = store.record_delivery(run_id, delivered, error) {
        error!(run_id, %store_error, "failed to record delivery outcome");
    }
}

/// The transport used when no bot token is configured: every send fails so the
/// missing configuration surfaces as a recorded failure, not a dropped result.
struct UnconfiguredSender;

#[async_trait]
impl MessageSender for UnconfiguredSender {
    async fn send(&self, _chat_id: i64, _text: &str) -> Result<(), AlfredError> {
        Err(AlfredError::Connector(
            "telegram bot_token is required to deliver job results".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(report: ReportPolicy) -> Job {
        Job {
            id: "job-1".to_string(),
            name: "digest".to_string(),
            kind: crate::jobs::JobKind::Recurring,
            schedule: Some("*/5 * * * *".to_string()),
            run_at: None,
            prompt: "ping".to_string(),
            report,
            deliver_to: None,
            model: None,
            tools: Vec::new(),
            timeout_secs: 900,
            enabled: true,
            last_run: None,
            last_status: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn end(verdict: Option<&str>) -> RunEnd {
        let mut end = RunEnd::new("success");
        end.verdict = verdict.map(str::to_string);
        end.output = Some("body".to_string());
        end
    }

    #[test]
    fn always_wants_every_run_and_on_signal_wants_only_match() {
        let always = job(ReportPolicy::Always);
        assert!(Delivery::wants_delivery(&always, &end(None)));
        assert!(Delivery::wants_delivery(&always, &end(Some("NO_MATCH"))));

        let on_signal = job(ReportPolicy::OnSignal);
        assert!(Delivery::wants_delivery(&on_signal, &end(Some(VERDICT_MATCH))));
        assert!(!Delivery::wants_delivery(&on_signal, &end(Some("NO_MATCH"))));
        assert!(!Delivery::wants_delivery(&on_signal, &end(None)));
    }

    #[test]
    fn deliver_to_wins_over_the_allowed_user_fallback() {
        let delivery = Delivery::new(Some("token".to_string()), &[42, 99]);
        let mut job = job(ReportPolicy::Always);
        job.deliver_to = Some("1234".to_string());
        assert_eq!(delivery.resolve_chat(&job).expect("valid"), Some(1234));

        job.deliver_to = None;
        assert_eq!(delivery.resolve_chat(&job).expect("fallback"), Some(42));
    }

    #[test]
    fn an_empty_allowed_list_has_no_recipient() {
        let delivery = Delivery::new(Some("token".to_string()), &[]);
        assert_eq!(delivery.resolve_chat(&job(ReportPolicy::Always)).expect("none"), None);
    }

    #[test]
    fn an_unparseable_deliver_to_is_a_configuration_error() {
        let delivery = Delivery::new(Some("token".to_string()), &[42]);
        let mut job = job(ReportPolicy::Always);
        job.deliver_to = Some("not-a-chat".to_string());
        let error = delivery
            .resolve_chat(&job)
            .expect_err("must reject an unparseable chat id");
        assert!(error.contains("not-a-chat"), "error was: {error}");
    }
}

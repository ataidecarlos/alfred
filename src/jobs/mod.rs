//! Job domain model.
//!
//! A job is a named prompt plus a schedule and a delivery policy. This module
//! owns the types, the validation rules applied on insert/update, and the cron
//! parsing used to decide when a job is due. Persistence lives in
//! [`crate::store::Store`]; the scheduler loop arrives in a later issue.
//!
//! [`runner`] turns a due job into a Pi run and reports a [`RunEnd`].

pub mod delivery;
pub mod dispatch;
pub mod runner;

use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};

use crate::error::AlfredError;

/// How a job decides when to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Runs exactly once, at `run_at`.
    Once,
    /// Runs on a recurring cron schedule.
    Recurring,
    /// Polls on a cron schedule, with a minimum interval.
    Watch,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Once => "once",
            JobKind::Recurring => "recurring",
            JobKind::Watch => "watch",
        }
    }
}

impl FromStr for JobKind {
    type Err = AlfredError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "once" => Ok(JobKind::Once),
            "recurring" => Ok(JobKind::Recurring),
            "watch" => Ok(JobKind::Watch),
            other => Err(AlfredError::JobValidation(format!(
                "unknown job kind '{other}'"
            ))),
        }
    }
}

/// When a job result is delivered to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportPolicy {
    /// Deliver every completed run.
    Always,
    /// Deliver only when the run reports a signal.
    OnSignal,
}

impl ReportPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportPolicy::Always => "always",
            ReportPolicy::OnSignal => "on_signal",
        }
    }
}

impl FromStr for ReportPolicy {
    type Err = AlfredError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "always" => Ok(ReportPolicy::Always),
            "on_signal" => Ok(ReportPolicy::OnSignal),
            other => Err(AlfredError::JobValidation(format!(
                "unknown report policy '{other}'"
            ))),
        }
    }
}

/// A persisted job row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    pub schedule: Option<String>,
    pub run_at: Option<i64>,
    pub prompt: String,
    pub report: ReportPolicy,
    pub deliver_to: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub last_run: Option<i64>,
    pub last_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The fields required to create (or fully replace) a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewJob {
    pub name: String,
    pub kind: JobKind,
    pub schedule: Option<String>,
    pub run_at: Option<i64>,
    pub prompt: String,
    pub report: ReportPolicy,
    pub deliver_to: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub timeout_secs: u64,
}

impl Default for NewJob {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: JobKind::Once,
            schedule: None,
            run_at: None,
            prompt: String::new(),
            report: ReportPolicy::OnSignal,
            deliver_to: None,
            model: None,
            tools: Vec::new(),
            timeout_secs: 900,
        }
    }
}

impl NewJob {
    /// Validate the job. `min_watch_interval_secs` is the configured floor for
    /// `watch` jobs.
    pub fn validate(&self, min_watch_interval_secs: u64) -> Result<(), AlfredError> {
        match self.kind {
            JobKind::Once => {
                if self.run_at.is_none() {
                    return Err(AlfredError::JobValidation(
                        "once job requires run_at".to_string(),
                    ));
                }
            }
            JobKind::Recurring | JobKind::Watch => {
                let expression = self.schedule.as_deref().ok_or_else(|| {
                    AlfredError::JobValidation(
                        "recurring and watch jobs require a schedule".to_string(),
                    )
                })?;
                let schedule = parse_five_field_cron(expression)?;
                if self.kind == JobKind::Watch {
                    let interval = cron_interval_secs(&schedule, Utc::now())?;
                    if interval < min_watch_interval_secs as i64 {
                        return Err(AlfredError::WatchIntervalTooShort(min_watch_interval_secs));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A persisted run of a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub job_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub verdict: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub cost_usd: Option<f64>,
    pub delivered: bool,
}

/// The terminal fields written by [`crate::store::Store::record_run_end`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunEnd {
    pub status: String,
    pub verdict: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub tokens_input: Option<i64>,
    pub tokens_output: Option<i64>,
    pub cost_usd: Option<f64>,
    pub delivered: bool,
}

impl RunEnd {
    pub fn new(status: impl Into<String>) -> Self {
        Self {
            status: status.into(),
            ..Self::default()
        }
    }
}

/// Parse a Unix five-field cron expression (`minute hour day-of-month month
/// day-of-week`).
///
/// The [`cron`] crate evaluates seconds-first expressions, so the seconds field
/// is pinned to `:00` before delegating to it. A non-five-field expression is
/// rejected outright; a malformed five-field one surfaces the parser's own
/// error message.
pub fn parse_five_field_cron(expression: &str) -> Result<Schedule, AlfredError> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(AlfredError::JobValidation(format!(
            "invalid cron expression '{expression}': expected 5 fields, found {}",
            fields.len()
        )));
    }
    let normalized = format!("0 {}", fields.join(" "));
    Schedule::from_str(&normalized).map_err(|error| {
        AlfredError::JobValidation(format!("invalid cron expression '{expression}': {error}"))
    })
}

/// The gap, in seconds, between the next two fires of `schedule` after `now`.
pub fn cron_interval_secs(schedule: &Schedule, now: DateTime<Utc>) -> Result<i64, AlfredError> {
    let mut upcoming = schedule.after(&now);
    let first = upcoming
        .next()
        .ok_or_else(|| AlfredError::JobValidation("cron expression never fires".to_string()))?;
    let second = upcoming
        .next()
        .ok_or_else(|| AlfredError::JobValidation("cron expression fires only once".to_string()))?;
    Ok((second - first).num_seconds())
}

/// Whether `job` is due at `now`.
///
/// `once` jobs run at most once. `recurring` and `watch` jobs are due when one
/// schedule period has elapsed since the last run; missed periods are not
/// backfilled.
pub fn is_due(job: &Job, now: i64) -> Result<bool, AlfredError> {
    if !job.enabled {
        return Ok(false);
    }
    match job.kind {
        JobKind::Once => Ok(job.last_run.is_none() && job.run_at.is_some_and(|at| at <= now)),
        JobKind::Recurring | JobKind::Watch => {
            let Some(expression) = job.schedule.as_deref() else {
                return Ok(false);
            };
            let schedule = parse_five_field_cron(expression)?;
            let baseline = job.last_run.unwrap_or(0);
            let baseline_dt = DateTime::<Utc>::from_timestamp(baseline, 0).ok_or_else(|| {
                AlfredError::JobValidation(format!("invalid last_run timestamp {baseline}"))
            })?;
            Ok(schedule
                .after(&baseline_dt)
                .next()
                .is_some_and(|next| next.timestamp() <= now))
        }
    }
}

/// Encode a job's tool list for the single `jobs.tools` TEXT column.
pub fn encode_tools(tools: &[String]) -> Option<String> {
    let joined: Vec<&str> = tools
        .iter()
        .map(|tool| tool.trim())
        .filter(|tool| !tool.is_empty())
        .collect();
    if joined.is_empty() {
        None
    } else {
        Some(joined.join(","))
    }
}

/// Decode the `jobs.tools` column back into a tool list.
pub fn decode_tools(raw: Option<String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_field_cron_parses_and_measures_interval() {
        let schedule = parse_five_field_cron("*/1 * * * *").expect("every minute is valid");
        let interval = cron_interval_secs(&schedule, Utc::now()).expect("periodic schedule");
        assert_eq!(interval, 60);
    }

    #[test]
    fn non_five_field_cron_is_rejected() {
        let error = parse_five_field_cron("* * *").unwrap_err();
        assert!(error.to_string().contains("expected 5 fields"), "{error}");
    }

    #[test]
    fn tool_list_round_trips() {
        let tools = vec!["bash".to_string(), " todo ".to_string(), String::new()];
        let encoded = encode_tools(&tools);
        assert_eq!(encoded.as_deref(), Some("bash,todo"));
        assert_eq!(
            decode_tools(encoded),
            vec!["bash".to_string(), "todo".to_string()]
        );
        assert_eq!(decode_tools(None), Vec::<String>::new());
    }
}

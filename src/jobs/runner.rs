//! The Pi-backed job runner.
//!
//! [`JobRunner`] implements [`Dispatch`]: the scheduler hands it a due job and
//! it turns that job into a one-shot Pi run, parses the run's verdict, and
//! reports a [`RunEnd`] for the scheduler to persist. The scheduler owns the run
//! row ([`crate::store::Store::record_run_start`] /
//! [`crate::store::Store::record_run_end`]); the runner never touches it.
//!
//! # Prompt assembly
//!
//! The Pi `--system-prompt` is the assembled persona from [`crate::prompt`]
//! (system + user context + memories). The job's own prompt is sent as the Pi
//! `prompt` command. When `report = on_signal`, the runner *also* appends the
//! verdict instruction to the Pi prompt so the last line of the assistant's
//! reply can carry a verdict.
//!
//! # The verdict contract
//!
//! For `on_signal` jobs the last line matching `(?i)^VERDICT:\s*(MATCH|NO_MATCH)\s*$`
//! is the verdict. A missing verdict is not an error: it falls back per
//! `missing_verdict` (`notify` → `MATCH`, logged at warn; `skip` → `NO_MATCH`).
//! Failing open is deliberate — silently dropping a possible alert is worse
//! than a false alert.
//!
//! # Failure is always terminal
//!
//! A run is never left `running`. Every error path — spawn failure, timeout,
//! non-zero exit, a Pi RPC error, missing assistant text — closes the run with
//! a terminal status and, where it exists, the failing detail:
//!
//! | condition                | status    |
//! |--------------------------|-----------|
//! | success                  | `success` |
//! | missing assistant text   | `failed`  |
//! | non-zero exit            | `failed`  |
//! | Pi RPC error             | `failed`  |
//! | spawn failure            | `failed`  |
//! | timeout                  | `timeout` |
//!
//! The child is reaped on every path: [`PiClient`] kills on drop, the timeout
//! arm kills and waits explicitly, and the exit-status probe reaps an
//! already-exited child.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::warn;

use crate::config::PiConfig;
use crate::error::AlfredError;
use crate::jobs::{Job, ReportPolicy, RunEnd};
use crate::pi::{PiClient, PiInvocation};
use crate::prompt::{assemble_system, load_prompt_layers};
use crate::scheduler::Dispatch;

/// Terminal status recorded for a run that succeeded.
pub const STATUS_SUCCESS: &str = "success";
/// Terminal status recorded for a run that failed for any non-timeout reason.
pub const STATUS_FAILED: &str = "failed";
/// Terminal status recorded when the run exceeded its timeout.
pub const STATUS_TIMEOUT: &str = "timeout";

/// Recorded when Pi produced no assistant text at all.
pub const ERR_NO_ASSISTANT_TEXT: &str = "no assistant text";

/// Verdict recorded when the assistant named a signal.
pub const VERDICT_MATCH: &str = "MATCH";
/// Verdict recorded when the assistant reported no signal.
pub const VERDICT_NO_MATCH: &str = "NO_MATCH";

/// The instruction appended to an `on_signal` job prompt.
pub const VERDICT_INSTRUCTION: &str =
    "End your reply with a final line containing exactly VERDICT: MATCH or VERDICT: NO_MATCH.";

/// How a missing verdict line is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingVerdict {
    /// Treat a missing verdict as [`VERDICT_MATCH`] and warn (fails open).
    Notify,
    /// Treat a missing verdict as [`VERDICT_NO_MATCH`].
    Skip,
}

impl MissingVerdict {
    /// Parse the `[jobs].missing_verdict` value.
    ///
    /// Only `notify` and `skip` are meaningful. Anything else falls back to the
    /// safe default, [`MissingVerdict::Notify`], and is logged: a typo in the
    /// config must not start silently dropping alerts.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "notify" => MissingVerdict::Notify,
            "skip" => MissingVerdict::Skip,
            other => {
                warn!(
                    value = %other,
                    "unknown jobs.missing_verdict; falling back to 'notify'"
                );
                MissingVerdict::Notify
            }
        }
    }
}

impl Default for MissingVerdict {
    fn default() -> Self {
        MissingVerdict::Notify
    }
}

/// A job runner backed by a one-shot `pi --mode rpc` subprocess.
///
/// The Pi binary/model/tools/timeout are supplied by the caller (issue #16
/// injects the loaded [`AppConfig`](crate::config::AppConfig)); the runner never
/// reads the config file itself. Tests point `pi.binary` at the compiled double.
pub struct JobRunner {
    pi: PiConfig,
    missing_verdict: MissingVerdict,
}

impl JobRunner {
    /// Build a runner from the `[pi]` config and the `[jobs].missing_verdict`
    /// fallback.
    pub fn new(pi: PiConfig, missing_verdict: MissingVerdict) -> Self {
        Self {
            pi,
            missing_verdict,
        }
    }

    /// The `[pi]` config this runner holds.
    pub fn pi(&self) -> &PiConfig {
        &self.pi
    }

    /// The verdict fallback this runner holds.
    pub fn missing_verdict(&self) -> MissingVerdict {
        self.missing_verdict
    }

    /// Build the exact Pi prompt for `job`: the job prompt, plus the verdict
    /// instruction when the report policy is [`ReportPolicy::OnSignal`].
    pub fn build_prompt(job: &Job) -> String {
        match job.report {
            ReportPolicy::OnSignal => {
                format!("{}\n\n{}", job.prompt.trim_end(), VERDICT_INSTRUCTION)
            }
            ReportPolicy::Always => job.prompt.clone(),
        }
    }

    /// Parse the verdict from the last line matching the verdict contract.
    ///
    /// Returns `Ok(Some(verdict))` for a bare `VERDICT: MATCH|NO_MATCH` line
    /// (case-insensitive, the last such line wins), `Ok(None)` when no line
    /// matches, and `Err(message)` for a malformed verdict token such as
    /// `VERDICT: MAYBE`. Interior lines are ignored: only the *whole* line must
    /// be the verdict.
    pub fn parse_verdict(output: &str) -> Result<Option<&'static str>, String> {
        let mut found = None;
        for line in output.lines() {
            let Some(rest) = strip_verdict_prefix(line) else {
                continue;
            };
            match rest.to_ascii_uppercase().as_str() {
                "MATCH" => found = Some(VERDICT_MATCH),
                "NO_MATCH" => found = Some(VERDICT_NO_MATCH),
                other => return Err(format!("malformed verdict line '{other}'")),
            }
        }
        Ok(found)
    }

    /// Parse the assistant output for `job` and resolve it into the verdict to
    /// persist, applying the missing-verdict policy.
    ///
    /// A malformed verdict token is an error, not a silent fallback: it means
    /// the model ignored the contract, and guessing which way to fail could
    /// drop an alert. Callers turn this into a `failed` run.
    pub fn verdict_for(&self, job: &Job, output: &str) -> Result<Option<String>, String> {
        if job.report == ReportPolicy::Always {
            // No verdict contract was sent; do not invent one.
            return Ok(None);
        }
        match Self::parse_verdict(output)? {
            Some(verdict) => Ok(Some(verdict.to_string())),
            None => match self.missing_verdict {
                MissingVerdict::Notify => {
                    warn!(
                        job = %job.name,
                        job_id = %job.id,
                        "job output has no verdict line; treating as MATCH (missing_verdict=notify)"
                    );
                    Ok(Some(VERDICT_MATCH.to_string()))
                }
                MissingVerdict::Skip => Ok(Some(VERDICT_NO_MATCH.to_string())),
            },
        }
    }

    /// Run `job` through a one-shot Pi subprocess and report how it ended.
    ///
    /// The assembled prompt is the `--system-prompt`; the job prompt is sent as
    /// the Pi `prompt` command. [`Dispatch::dispatch`] is the trait entry point;
    /// this inherent method is what it delegates to so callers can drive it
    /// without importing the trait.
    pub async fn run(&self, job: &Job) -> Result<RunEnd, AlfredError> {
        let layers = load_prompt_layers(&self.pi_prompt_config())?;
        let system_prompt = assemble_system(&layers);

        let mut pi_config = self.pi.clone();
        if let Some(model) = job.model.as_deref().filter(|model| !model.is_empty()) {
            pi_config.model = model.to_string();
        }
        if !job.tools.is_empty() {
            pi_config.jobs_tools = job.tools.clone();
        }
        let binary = pi_config.binary.clone();

        let invocation = PiInvocation::job(&pi_config, system_prompt);
        let prompt = Self::build_prompt(job);

        let timeout_secs = effective_timeout_secs(&pi_config, job);
        let run = self
            .run_pi(&binary, invocation.command(), &prompt, timeout_secs)
            .await;

        self.finish(job, run)
    }

    /// Validate the invocation boundary without spawning Pi.
    ///
    /// Exposed so integration tests can prove the exact `--system-prompt` and
    /// prompt body without a live model. Returns the assembled system prompt and
    /// the prompt that would be sent.
    pub fn prepare(&self, job: &Job) -> Result<(String, String), AlfredError> {
        let layers = load_prompt_layers(&self.pi_prompt_config())?;
        let system_prompt = assemble_system(&layers);
        Ok((system_prompt, Self::build_prompt(job)))
    }

    /// Assemble the outcome of a Pi attempt into a terminal [`RunEnd`].
    ///
    /// This is the single mapping from "how the Pi attempt ended" to "what the
    /// run row says", so it is exposed for tests: they can drive a written-out
    /// exit code, timeout, or output through the real mapping without needing
    /// the fixed double to produce it.
    pub fn finish_run(&self, job: &Job, run: Result<PiCompletion, AlfredError>) -> RunEnd {
        let run = run.map(PiAttempt::from);
        match self.finish(job, run) {
            Ok(end) => end,
            Err(error) => self.failed_end_from_error(job, error),
        }
    }

    /// Assemble the outcome of a Pi attempt into a terminal [`RunEnd`].
    fn finish(
        &self,
        job: &Job,
        run: Result<PiAttempt, AlfredError>,
    ) -> Result<RunEnd, AlfredError> {
        match run {
            Ok(attempt) if attempt.timed_out => Ok(self.timeout_end(&attempt)),
            Ok(attempt) => match attempt.exit.success() {
                Some(false) => Ok(self.failed_end(&attempt, describe_exit(&attempt.exit))),
                _ => self.success_or_missing_text(job, attempt),
            },
            Err(error) => Ok(self.failed_end_from_error(job, error)),
        }
    }

    /// Build the `success` end, or a `failed` end when Pi produced no text.
    fn success_or_missing_text(
        &self,
        job: &Job,
        attempt: PiAttempt,
    ) -> Result<RunEnd, AlfredError> {
        let PiAttempt {
            output,
            stats,
            exit,
            ..
        } = attempt;
        let output = match output {
            Some(text) if !text.trim().is_empty() => text,
            _ => {
                return Ok(failed_end(
                    ERR_NO_ASSISTANT_TEXT.to_string(),
                    None,
                    Some(describe_exit(&exit)),
                ));
            }
        };

        let verdict = match self.verdict_for(job, &output) {
            Ok(verdict) => verdict,
            Err(message) => {
                return Ok(self.failed_end_at(
                    &format!("unparseable verdict: {message}"),
                    Some(output),
                    &exit,
                ));
            }
        };

        let mut end = RunEnd::new(STATUS_SUCCESS);
        end.verdict = verdict;
        end.output = Some(output);
        if let Some(stats) = stats {
            end.tokens_input = stats.input;
            end.tokens_output = stats.output;
            end.cost_usd = stats.cost;
        }
        Ok(end)
    }

    fn timeout_end(&self, attempt: &PiAttempt) -> RunEnd {
        let mut end = RunEnd::new(STATUS_TIMEOUT);
        end.error = Some(format!("Pi run exceeded {} seconds", attempt.timeout_secs));
        apply_stats(&mut end, attempt);
        end
    }

    fn failed_end(&self, attempt: &PiAttempt, message: String) -> RunEnd {
        let mut end = RunEnd::new(STATUS_FAILED);
        end.output = attempt.output.clone();
        end.error = Some(message);
        apply_stats(&mut end, attempt);
        end
    }

    fn failed_end_at(&self, message: &str, output: Option<String>, exit: &ExitStatus) -> RunEnd {
        failed_end(message.to_string(), output, Some(describe_exit(exit)))
    }

    fn failed_end_from_error(&self, job: &Job, error: AlfredError) -> RunEnd {
        let message = error.to_string();
        warn!(job = %job.name, job_id = %job.id, error = %message, "Pi run failed");
        RunEnd {
            status: STATUS_FAILED.to_string(),
            error: Some(message),
            ..RunEnd::default()
        }
    }

    /// Drive the whole RPC exchange in one cancellable future.
    async fn run_pi(
        &self,
        binary: &str,
        command: tokio::process::Command,
        prompt: &str,
        timeout_secs: u64,
    ) -> Result<PiAttempt, AlfredError> {
        let outcome = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            self.converse(binary, command, prompt),
        )
        .await;

        match outcome {
            Ok(result) => result,
            Err(_elapsed) => {
                warn!(binary = %binary, timeout_secs, "Pi run exceeded its timeout; killing the child");
                // The future is abandoned here, so its `PiClient` is dropped and
                // `kill_on_drop` reaps the child. Reporting the attempt as timed
                // out is what records `timeout`.
                Ok(PiAttempt {
                    output: None,
                    stats: None,
                    exit: ExitStatus::Unknown,
                    timed_out: true,
                    timeout_secs,
                })
            }
        }
    }

    /// One prompt exchange: spawn, prompt, gather events, ask for the text and
    /// the session stats, then reap the child.
    async fn converse(
        &self,
        binary: &str,
        command: tokio::process::Command,
        prompt: &str,
    ) -> Result<PiAttempt, AlfredError> {
        let mut client = PiClient::spawn(binary, command).await?;

        let accepted = client
            .request(json!({"type": "prompt", "message": prompt}))
            .await?;
        if !accepted.success {
            return Err(AlfredError::Pi(format!(
                "Pi rejected the prompt: {}",
                accepted
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            )));
        }

        // Drain the stream until the agent settles. Streamed records are the
        // assistant text surface, but the authoritative text is fetched with
        // `get_last_assistant_text` after settling.
        let mut settled = false;
        for _ in 0..100_000 {
            let message = client.next_message().await?;
            if message["type"].as_str() == Some("agent_settled") {
                settled = true;
                break;
            }
        }
        if !settled {
            return Err(AlfredError::Pi(
                "Pi stream ended without agent_settled".to_string(),
            ));
        }

        let text = client
            .request(json!({"type": "get_last_assistant_text"}))
            .await?;
        let output = if text.success {
            match text.data.as_ref() {
                Some(Value::Null) | None => None,
                Some(data) => data.get("text").and_then(Value::as_str).map(str::to_string),
            }
        } else {
            return Err(AlfredError::Pi(format!(
                "get_last_assistant_text failed: {}",
                text.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        };
        let stats = self.session_stats(&mut client).await;
        let exit = reap(&mut client).await;

        Ok(PiAttempt {
            output,
            stats,
            exit,
            timed_out: false,
            timeout_secs: 0,
        })
    }

    /// Best-effort session stats. A failure leaves them unset for the run row.
    async fn session_stats(&self, client: &mut PiClient) -> Option<PiStats> {
        let response = client
            .request(json!({"type": "get_session_stats"}))
            .await
            .ok()?;
        if !response.success {
            warn!(
                error = response.error.unwrap_or_default(),
                "get_session_stats failed; recording no tokens or cost"
            );
            return None;
        }
        let data = response.data?;
        let tokens = data.get("tokens");
        Some(PiStats {
            input: tokens
                .and_then(|tokens| tokens.get("input"))
                .and_then(Value::as_i64),
            output: tokens
                .and_then(|tokens| tokens.get("output"))
                .and_then(Value::as_i64),
            cost: data.get("cost").and_then(Value::as_f64),
        })
    }

    /// The minimal [`PromptConfig`](crate::config::PromptConfig) the prompt
    /// loader needs. The runner reads prompt/memory files from their default
    /// locations; only the `[pi]` config is injected.
    fn pi_prompt_config(&self) -> crate::config::PromptConfig {
        crate::config::PromptConfig {
            system_prompt_file: crate::paths::Paths::system_prompt_file()
                .to_string_lossy()
                .into_owned(),
            user_prompt_file: crate::paths::Paths::user_prompt_file()
                .to_string_lossy()
                .into_owned(),
        }
    }
}

#[async_trait]
impl Dispatch for JobRunner {
    async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError> {
        self.run(job).await
    }
}

impl std::fmt::Debug for JobRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobRunner")
            .field("pi", &self.pi)
            .field("missing_verdict", &self.missing_verdict)
            .finish()
    }
}

/// The fields the run row needs from a Pi attempt.
#[derive(Debug)]
struct PiAttempt {
    output: Option<String>,
    stats: Option<PiStats>,
    exit: ExitStatus,
    timed_out: bool,
    timeout_secs: u64,
}

/// A caller-described Pi completion.
///
/// The runner maps a real subprocess's outcome into this shape internally. It is
/// public so a test can describe an outcome the deterministic double cannot
/// produce (a non-zero exit, a timeout, a chosen assistant body) and drive it
/// through the exact same terminal mapping the live path uses.
#[derive(Debug, Clone)]
pub struct PiCompletion {
    /// The last assistant text, if any.
    pub output: Option<String>,
    /// `(tokens_input, tokens_output, cost_usd)` from `get_session_stats`.
    pub stats: (Option<i64>, Option<i64>, Option<f64>),
    /// The child's exit code; `Some(0)` means success.
    pub exit_code: Option<i32>,
    /// Whether the run was killed for exceeding its timeout.
    pub timed_out: bool,
    /// The timeout the run exceeded, when [`PiCompletion::timed_out`].
    pub timeout_secs: u64,
}

impl From<PiCompletion> for PiAttempt {
    fn from(completion: PiCompletion) -> Self {
        let (input, output, cost) = completion.stats;
        Self {
            output: completion.output,
            stats: Some(PiStats {
                input,
                output,
                cost,
            }),
            exit: match completion.exit_code {
                Some(0) => ExitStatus::Success,
                Some(code) => ExitStatus::Code(code.to_string()),
                None => ExitStatus::Unknown,
            },
            timed_out: completion.timed_out,
            timeout_secs: completion.timeout_secs,
        }
    }
}

/// Token/cost figures from `get_session_stats`.
#[derive(Debug, Clone, Copy, Default)]
struct PiStats {
    input: Option<i64>,
    output: Option<i64>,
    cost: Option<f64>,
}

/// Whether Pi had exited when the run finished, and with what.
#[derive(Debug, Clone)]
enum ExitStatus {
    Success,
    Code(String),
    Unknown,
}

impl ExitStatus {
    fn success(&self) -> Option<bool> {
        match self {
            ExitStatus::Success => Some(true),
            ExitStatus::Code(_) => Some(false),
            ExitStatus::Unknown => None,
        }
    }
}

/// Reap the child, mapping its exit status. A child that exited by the time the
/// conversation closed is reaped here; a still-running child is killed and
/// reaped so no process is left behind.
async fn reap(client: &mut PiClient) -> ExitStatus {
    match client.try_wait() {
        Ok(Some(status)) => exit_status(status),
        _ => {
            client.kill().await;
            ExitStatus::Unknown
        }
    }
}

fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    if status.success() {
        ExitStatus::Success
    } else {
        ExitStatus::Code(status.to_string())
    }
}

/// A human-readable one-line description of how Pi exited, used in error text.
fn describe_exit(exit: &ExitStatus) -> String {
    match exit {
        ExitStatus::Success => "Pi exited successfully".to_string(),
        ExitStatus::Code(code) => format!("Pi exited with status {code}"),
        ExitStatus::Unknown => "Pi exit status unknown".to_string(),
    }
}

fn failed_end(message: String, output: Option<String>, exit: Option<String>) -> RunEnd {
    let mut end = RunEnd::new(STATUS_FAILED);
    end.output = output;
    end.error = Some(match exit {
        Some(exit) => format!("{message}; {exit}"),
        None => message,
    });
    end
}

fn apply_stats(end: &mut RunEnd, attempt: &PiAttempt) {
    if let Some(stats) = attempt.stats {
        end.tokens_input = stats.input;
        end.tokens_output = stats.output;
        end.cost_usd = stats.cost;
    }
}

/// The timeout for `job`, falling back to the `[pi]` timeout.
///
/// `0` is never a valid timeout; treat it as "unset" and use the Pi default so
/// a misconfigured job cannot kill itself instantly.
fn effective_timeout_secs(pi: &PiConfig, job: &Job) -> u64 {
    if job.timeout_secs > 0 {
        job.timeout_secs
    } else if pi.timeout_secs > 0 {
        pi.timeout_secs
    } else {
        crate::config::PiConfig::default().timeout_secs
    }
}

/// Strip a leading `VERDICT:` (case-insensitive), returning the trimmed rest.
fn strip_verdict_prefix(line: &str) -> Option<&str> {
    let line = line.trim();
    let (key, value) = line.split_once(':')?;
    if key.trim().eq_ignore_ascii_case("VERDICT") {
        Some(value.trim())
    } else {
        None
    }
}

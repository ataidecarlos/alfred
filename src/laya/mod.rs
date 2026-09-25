//! Laya decision layer.
//!
//! Laya is a small local decision model that scores an inbound request with a
//! calibrated confidence. The routing policy is:
//!
//! * `confidence > threshold` → execute directly, without calling the LLM
//! * `confidence <= threshold` → delegate to the configured LLM
//!
//! The threshold defaults to [`DEFAULT_CONFIDENCE_THRESHOLD`] (0.8) and the
//! boundary is exclusive: a confidence of exactly the threshold delegates.
//! Every decision is logged (via `tracing`) and kept in a bounded in-memory
//! log.
//!
//! The upstream Laya model (`convaiinnovations/laya`) is a 421M
//! text-classification model intended to run locally. This build ships a
//! deterministic, dependency-free scorer in [`LayaModel::score`] so the routing
//! policy, decision log, and call-site integration are testable without a
//! model runtime. A real classifier backend can replace `score` without
//! touching the call sites.

use std::collections::VecDeque;
use std::sync::Mutex;

use chrono::Utc;
use tracing::info;

use crate::types::{extract_text, AssistantMessage, ContentBlock, Message, StopReason, Usage};

/// Requests with a confidence strictly greater than this execute directly.
pub const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.8;

/// Model name recorded on assistant messages produced by the direct path.
pub const LAYA_MODEL_NAME: &str = "laya";

/// How many recent decisions are retained in memory.
const MAX_DECISION_LOG: usize = 256;

const HELP_TEXT: &str =
    "Alfred help: send a message and I'll handle it. Built-in tools: shell, webhook, todo.";

/// Which path a request should take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionPath {
    /// High confidence: answer directly, no LLM call.
    ExecuteDirect,
    /// Low confidence: hand the request to the LLM.
    DelegateToLlm,
}

impl DecisionPath {
    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionPath::ExecuteDirect => "execute_direct",
            DecisionPath::DelegateToLlm => "delegate_to_llm",
        }
    }
}

/// The confidence score and chosen path for a single request.
#[derive(Debug, Clone, PartialEq)]
pub struct LayaDecision {
    pub confidence: f32,
    pub path: DecisionPath,
    /// Fast-path intent label when one matched, e.g. `"ping"`.
    pub intent: Option<String>,
    /// Human-readable explanation of the score.
    pub reason: String,
}

/// Confidence-based request router.
pub struct LayaModel {
    threshold: f32,
    decisions: Mutex<VecDeque<LayaDecision>>,
}

impl Default for LayaModel {
    fn default() -> Self {
        Self::new(DEFAULT_CONFIDENCE_THRESHOLD)
    }
}

impl LayaModel {
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold,
            decisions: Mutex::new(VecDeque::new()),
        }
    }

    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Map a confidence score to a decision path. The threshold is exclusive:
    /// exactly `threshold` delegates to the LLM.
    pub fn path_for_confidence(&self, confidence: f32) -> DecisionPath {
        if confidence > self.threshold {
            DecisionPath::ExecuteDirect
        } else {
            DecisionPath::DelegateToLlm
        }
    }

    /// Score a request, record and log the resulting decision.
    pub fn decide(&self, request: &str) -> LayaDecision {
        let (confidence, intent, reason) = self.score(request);
        let decision = LayaDecision {
            confidence,
            path: self.path_for_confidence(confidence),
            intent,
            reason,
        };

        info!(
            confidence = decision.confidence,
            threshold = self.threshold,
            path = decision.path.as_str(),
            reason = %decision.reason,
            "laya decision"
        );

        if let Ok(mut log) = self.decisions.lock() {
            if log.len() == MAX_DECISION_LOG {
                log.pop_front();
            }
            log.push_back(decision.clone());
        }

        decision
    }

    /// Decide and, when the request is confident enough to run directly,
    /// produce the direct reply. Returns `None` when the request must be
    /// delegated to the LLM.
    pub fn decide_and_respond(&self, request: &str) -> Option<String> {
        let decision = self.decide(request);
        if decision.path == DecisionPath::ExecuteDirect {
            direct_response(request)
        } else {
            None
        }
    }

    /// Snapshot of the retained decisions, oldest first.
    pub fn decisions(&self) -> Vec<LayaDecision> {
        self.decisions
            .lock()
            .map(|log| log.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Deterministic stand-in for the Laya classifier. Returns the confidence,
    /// a fast-path intent label when one matched, and a human-readable reason.
    fn score(&self, request: &str) -> (f32, Option<String>, String) {
        let normalized = normalize(request);
        if normalized.is_empty() {
            return (0.0, None, "empty request".to_string());
        }

        if let Some((intent, _)) = fast_path(&normalized) {
            return (
                0.95,
                Some(intent.to_string()),
                format!("matched fast-path intent '{intent}'"),
            );
        }

        let word_count = normalized.split_whitespace().count();
        if word_count > 30 {
            return (0.2, None, "long request needs LLM reasoning".to_string());
        }
        if contains_reasoning_cue(&normalized) {
            return (
                0.25,
                None,
                "reasoning or creative cue needs the LLM".to_string(),
            );
        }
        if word_count <= 2 {
            return (0.3, None, "short ambiguous request".to_string());
        }

        (0.45, None, "no fast-path match; delegate to LLM".to_string())
    }
}

/// Resolve the direct reply for a request, if the fast path handles it.
pub fn direct_response(request: &str) -> Option<String> {
    fast_path(&normalize(request)).map(|(_, reply)| reply.to_string())
}

/// Find the most recent user message text, if any.
fn last_user_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|m| match m {
        Message::User(_) => {
            let text = extract_text(m);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    })
}

/// Build a direct assistant reply for the latest user message when Laya is
/// confident enough to answer without the LLM. Returns `None` to delegate.
pub fn direct_reply(model: &LayaModel, messages: &[Message]) -> Option<Message> {
    let request = last_user_text(messages)?;
    let text = model.decide_and_respond(&request)?;
    Some(Message::Assistant(AssistantMessage {
        content: vec![ContentBlock::Text { text }],
        usage: Usage {
            input: 0,
            output: 0,
            total: 0,
        },
        stop_reason: StopReason::Stop,
        model: LAYA_MODEL_NAME.to_string(),
        timestamp: Utc::now(),
    }))
}

/// Lowercase, trim, and drop trailing sentence punctuation for matching.
fn normalize(request: &str) -> String {
    request
        .trim()
        .trim_end_matches(['.', '!', '?'])
        .to_lowercase()
}

/// Fast-path intents Laya can answer directly, as `(intent, reply)`.
fn fast_path(normalized: &str) -> Option<(&'static str, &'static str)> {
    match normalized {
        "ping" => Some(("ping", "pong")),
        "version" => Some(("version", concat!("Alfred ", env!("CARGO_PKG_VERSION")))),
        "help" => Some(("help", HELP_TEXT)),
        _ => None,
    }
}

/// Cues that indicate the request needs real reasoning or generation.
fn contains_reasoning_cue(normalized: &str) -> bool {
    const CUES: &[&str] = &[
        "explain", "why", "how", "analyze", "analyse", "compare", "design", "implement",
        "debug", "write", "plan", "summarize", "summarise", "translate", "draft",
    ];
    normalized
        .split_whitespace()
        .any(|word| CUES.contains(&word.trim_matches(|c: char| !c.is_alphanumeric())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Message {
        Message::User(crate::types::UserMessage {
            content: vec![crate::types::text_content(text)],
            timestamp: Utc::now(),
        })
    }

    #[test]
    fn threshold_boundary_is_exclusive() {
        let model = LayaModel::default();
        assert_eq!(
            model.path_for_confidence(DEFAULT_CONFIDENCE_THRESHOLD),
            DecisionPath::DelegateToLlm
        );
        assert_eq!(model.path_for_confidence(0.79), DecisionPath::DelegateToLlm);
        assert_eq!(model.path_for_confidence(0.81), DecisionPath::ExecuteDirect);
    }

    #[test]
    fn fast_path_scores_high_and_delegates_nothing() {
        let model = LayaModel::default();
        let decision = model.decide("ping");
        assert_eq!(decision.path, DecisionPath::ExecuteDirect);
        assert_eq!(decision.intent.as_deref(), Some("ping"));
        assert_eq!(model.decide_and_respond("PING!"), Some("pong".to_string()));
    }

    #[test]
    fn complex_request_delegates() {
        let model = LayaModel::default();
        let decision = model.decide("Explain why the sky is blue in detail");
        assert_eq!(decision.path, DecisionPath::DelegateToLlm);
        assert!(decision.confidence <= DEFAULT_CONFIDENCE_THRESHOLD);
    }

    #[test]
    fn empty_request_delegates() {
        let model = LayaModel::default();
        let decision = model.decide("   ");
        assert_eq!(decision.path, DecisionPath::DelegateToLlm);
        assert_eq!(decision.confidence, 0.0);
    }

    #[test]
    fn decisions_are_recorded_in_order() {
        let model = LayaModel::default();
        model.decide("ping");
        model.decide("Explain quantum entanglement");
        let logged = model.decisions();
        assert_eq!(logged.len(), 2);
        assert_eq!(logged[0].path, DecisionPath::ExecuteDirect);
        assert_eq!(logged[1].path, DecisionPath::DelegateToLlm);
    }

    #[test]
    fn direct_reply_answers_only_the_latest_user_message() {
        let model = LayaModel::default();
        let messages = vec![user("ping"), user("Explain the theory of relativity")];
        assert!(direct_reply(&model, &messages).is_none());
    }

    #[test]
    fn version_reply_mentions_crate_version() {
        let model = LayaModel::default();
        let reply = model.decide_and_respond("version").unwrap();
        assert!(reply.contains(env!("CARGO_PKG_VERSION")));
    }
}

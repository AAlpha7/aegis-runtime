//! OpenRouter Decisions API client for TypeSafe Jev (System One).
//!
//! This is **not** chat completions. Endpoint:
//! `POST https://openrouter.ai/api/alpha/decisions`
//!
//! Models: `typesafe/jev-1.13` (pinned) or `~typesafe/jev-latest` (alias).
//! Request/response shapes follow TypeSafe System One (Choice / Score / Noul).

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default pinned Jev model on OpenRouter.
pub const DEFAULT_JEV_MODEL: &str = "typesafe/jev-1.13";

/// Floating alias that tracks the latest Jev revision on OpenRouter.
pub const JEV_LATEST_ALIAS: &str = "~typesafe/jev-latest";

/// OpenRouter Decisions endpoint (alpha). Not under `/api/v1/chat/completions`.
pub const OPENROUTER_DECISIONS_URL: &str = "https://openrouter.ai/api/alpha/decisions";

/// Which Jev model slug to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JevModel {
    /// Pinned `typesafe/jev-1.13`.
    Pinned,
    /// Alias `~typesafe/jev-latest`.
    Latest,
    /// Explicit override (tests / operator pin).
    Custom(String),
}

impl JevModel {
    pub fn from_env() -> Self {
        match std::env::var("AEGIS_JEV_MODEL") {
            Ok(v) => {
                let t = v.trim();
                if t.is_empty() || t == DEFAULT_JEV_MODEL {
                    Self::Pinned
                } else if t == JEV_LATEST_ALIAS || t == "jev-latest" || t == "latest" {
                    Self::Latest
                } else {
                    Self::Custom(t.to_string())
                }
            }
            Err(_) => Self::Pinned,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Pinned => DEFAULT_JEV_MODEL,
            Self::Latest => JEV_LATEST_ALIAS,
            Self::Custom(s) => s.as_str(),
        }
    }
}

/// One typed question in a Decisions request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionsQuestion {
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<BTreeMap<String, Option<String>>>,
    },
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Option<String>>,
    },
    Score {
        instructions: Value,
        criteria: Vec<String>,
    },
}

impl DecisionsQuestion {
    /// Choice over allow / ask / deny for tool gating.
    pub fn tool_gate_choice(instructions: impl Into<String>) -> Self {
        let mut criteria = BTreeMap::new();
        criteria.insert(
            "allow".to_string(),
            Some("Safe to execute without further owner confirmation.".into()),
        );
        criteria.insert(
            "ask".to_string(),
            Some("Needs explicit owner confirmation before execution.".into()),
        );
        criteria.insert(
            "deny".to_string(),
            Some("Must not execute; policy or risk forbids it.".into()),
        );
        Self::Choice {
            instructions: Value::String(instructions.into()),
            criteria,
        }
    }

    /// Noul: is this a high-risk computer-use action?
    pub fn high_risk_noul(instructions: impl Into<String>) -> Self {
        let mut criteria = BTreeMap::new();
        criteria.insert(
            "true".to_string(),
            Some("Destructive, exfiltrating, or privilege-escalating.".into()),
        );
        criteria.insert(
            "false".to_string(),
            Some("Read-only or otherwise low-risk.".into()),
        );
        Self::Noul {
            instructions: Value::String(instructions.into()),
            criteria: Some(criteria),
        }
    }

    /// Score: graded risk 0..=3.
    pub fn risk_score(instructions: impl Into<String>) -> Self {
        Self::Score {
            instructions: Value::String(instructions.into()),
            criteria: vec![
                "Negligible risk".into(),
                "Low risk".into(),
                "Elevated risk — prefer ask".into(),
                "Severe risk — prefer deny".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionsRequest {
    pub model: String,
    pub state: Value,
    pub questions: BTreeMap<String, DecisionsQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionsAnswer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        #[serde(default)]
        legend: BTreeMap<String, String>,
        #[serde(default)]
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        confidence: Option<f64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionsUsage {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DecisionsResponse {
    pub model: String,
    pub answers: BTreeMap<String, DecisionsAnswer>,
    #[serde(default)]
    pub usage: Option<DecisionsUsage>,
    #[serde(default)]
    pub id: Option<String>,
}

/// Thin HTTP client for OpenRouter Decisions.
#[derive(Debug, Clone)]
pub struct DecisionsClient {
    http: reqwest::Client,
    api_key: String,
    url: String,
    model: JevModel,
}

impl DecisionsClient {
    pub fn new(api_key: impl Into<String>, model: JevModel) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building Decisions HTTP client")?;
        Ok(Self {
            http,
            api_key: api_key.into(),
            url: OPENROUTER_DECISIONS_URL.to_string(),
            model,
        })
    }

    /// Override endpoint (tests / self-hosted mirrors).
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    pub fn model(&self) -> &JevModel {
        &self.model
    }

    pub async fn decide(&self, request: DecisionsRequest) -> Result<DecisionsResponse> {
        let resp = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://github.com/AAlpha7/aegis-runtime")
            .header("X-OpenRouter-Title", "Aegis Runtime Secure Computer Use")
            .json(&request)
            .send()
            .await
            .context("OpenRouter Decisions request failed")?;

        let status = resp.status();
        let body_text = resp.text().await.context("reading Decisions response")?;
        if !status.is_success() {
            bail!("OpenRouter Decisions HTTP {status}: {body_text}");
        }

        serde_json::from_str(&body_text).with_context(|| {
            format!("parsing Decisions response JSON (first 240 chars): {}", {
                let mut s = body_text.clone();
                s.truncate(240);
                s
            })
        })
    }

    /// Build a standard policy request for a CU tool call.
    pub fn policy_request(&self, state: Value) -> DecisionsRequest {
        let mut questions = BTreeMap::new();
        questions.insert(
            "verdict".to_string(),
            DecisionsQuestion::tool_gate_choice(
                "Given the computer-use intent in `state`, choose allow, ask, or deny.",
            ),
        );
        questions.insert(
            "high_risk".to_string(),
            DecisionsQuestion::high_risk_noul(
                "Is this computer-use action high-risk (destructive, exfil, privilege escalation)?",
            ),
        );
        questions.insert(
            "risk".to_string(),
            DecisionsQuestion::risk_score(
                "How severe is the risk of executing this computer-use intent?",
            ),
        );
        DecisionsRequest {
            model: self.model.as_str().to_string(),
            state,
            questions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_policy_request_shape() {
        let client = DecisionsClient::new("sk-or-test-key-1234567890", JevModel::Pinned).unwrap();
        let req = client.policy_request(json!({
            "tool": "system.frontmost_app",
            "args": {},
            "device_id": "dev-loopback-1",
        }));
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["model"], DEFAULT_JEV_MODEL);
        assert_eq!(v["questions"]["verdict"]["type"], "choice");
        assert_eq!(v["questions"]["high_risk"]["type"], "noul");
        assert_eq!(v["questions"]["risk"]["type"], "score");
        assert!(v["questions"]["verdict"]["criteria"]["allow"].is_string());
    }

    #[test]
    fn parses_choice_noul_score_answers() {
        let raw = json!({
            "model": "jev-1.13.0",
            "answers": {
                "verdict": {
                    "type": "choice",
                    "choice": "ask",
                    "probabilities": {"allow": 0.1, "ask": 0.8, "deny": 0.1},
                    "confidence": 0.7
                },
                "high_risk": {"type": "noul", "noul": 0.2},
                "risk": {
                    "type": "score",
                    "score": 1.1,
                    "legend": {"0": "Negligible risk", "1": "Low risk"},
                    "probabilities": {"0": 0.1, "1": 0.8, "2": 0.1},
                    "confidence": 0.75
                }
            },
            "usage": {"input_tokens": 100, "output_tokens": 20}
        });
        let parsed: DecisionsResponse = serde_json::from_value(raw).unwrap();
        match &parsed.answers["verdict"] {
            DecisionsAnswer::Choice { choice, .. } => assert_eq!(choice, "ask"),
            other => panic!("expected choice, got {other:?}"),
        }
        match &parsed.answers["high_risk"] {
            DecisionsAnswer::Noul { noul } => assert!((*noul - 0.2).abs() < f64::EPSILON),
            other => panic!("expected noul, got {other:?}"),
        }
    }

    #[test]
    fn jev_model_from_alias() {
        assert_eq!(JevModel::Latest.as_str(), JEV_LATEST_ALIAS);
        assert_eq!(JevModel::Pinned.as_str(), DEFAULT_JEV_MODEL);
    }
}

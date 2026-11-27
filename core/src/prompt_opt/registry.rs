use serde::{Deserialize, Serialize};

use super::model::PromptOptError;

/// Owner or gate outcome stored for the next analyst prompt.
/// This is not a Hindsight memory record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionSignalKind {
    Reject,
    Rollback,
    DistillRejected,
    ValidationRejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionSignal {
    pub kind: EvolutionSignalKind,
    pub version_id: u64,
    pub detail: String,
}

/// Feedback that conflicts with the current skill. The queue is a stub:
/// it does not retune the instruction and it does not write Hindsight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContradictionItem {
    pub id: u64,
    pub feedback: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackOutcome {
    Noted,
    ContradictionQueued { id: u64 },
}

/// Versioned instruction text. The live pointer moves only through
/// [`PromptRegistry::accept`] or a rollback to the baseline / an already
/// accepted version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptVersion {
    pub id: u64,
    pub instruction: String,
    pub parent_id: Option<u64>,
    pub score: Option<f64>,
    pub dimension_scores: Vec<(String, f64)>,
    pub notes: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptRegistry {
    versions: Vec<PromptVersion>,
    baseline_id: u64,
    live_id: u64,
    /// Validation winner. Moves only when a candidate scores strictly higher
    /// than this version. It does not make that version live.
    #[serde(default = "default_validation_best_id")]
    best_validation_id: u64,
    next_id: u64,
    #[serde(default)]
    evolution: Vec<EvolutionSignal>,
    #[serde(default)]
    contradictions: Vec<ContradictionItem>,
    #[serde(default = "default_next_contradiction_id")]
    next_contradiction_id: u64,
}

fn default_next_contradiction_id() -> u64 {
    1
}

fn default_validation_best_id() -> u64 {
    1
}

impl PromptRegistry {
    pub fn new(baseline_instruction: impl Into<String>) -> Self {
        let baseline = PromptVersion {
            id: 1,
            instruction: baseline_instruction.into(),
            parent_id: None,
            score: None,
            dimension_scores: Vec::new(),
            notes: "baseline".to_string(),
            accepted: true,
        };
        Self {
            versions: vec![baseline],
            baseline_id: 1,
            live_id: 1,
            best_validation_id: 1,
            next_id: 2,
            evolution: Vec::new(),
            contradictions: Vec::new(),
            next_contradiction_id: 1,
        }
    }

    pub fn baseline_id(&self) -> u64 {
        self.baseline_id
    }

    pub fn live_id(&self) -> u64 {
        self.live_id
    }

    pub fn validation_best_id(&self) -> u64 {
        self.best_validation_id
    }

    pub fn get(&self, id: u64) -> Option<&PromptVersion> {
        self.versions.iter().find(|version| version.id == id)
    }

    pub fn live(&self) -> &PromptVersion {
        self.get(self.live_id)
            .expect("live version is always present")
    }

    pub fn baseline(&self) -> &PromptVersion {
        self.get(self.baseline_id)
            .expect("baseline version is always present")
    }

    pub fn validation_best(&self) -> &PromptVersion {
        self.get(self.best_validation_id)
            .expect("validation best is always present")
    }

    /// Point `best_skill` at `id` only when its validation score is strictly
    /// higher than the current winner. An unscored winner loses to the first
    /// finite score. Equal scores stay put. This does not accept the version
    /// and does not move the live pointer.
    pub fn promote_if_strictly_better(&mut self, id: u64) -> Result<bool, PromptOptError> {
        let Some(candidate) = self.get(id) else {
            return Err(PromptOptError::invalid(format!(
                "unknown prompt version {id}"
            )));
        };
        let Some(candidate_score) = candidate.score else {
            return Ok(false);
        };
        let best_score = self.validation_best().score;
        let better = match best_score {
            Some(best) => candidate_score > best + 1e-9,
            None => true,
        };
        if better {
            self.best_validation_id = id;
        }
        Ok(better)
    }

    /// Registry artifact equivalent of `best_skill.md`. The text is the
    /// validation winner. It stays off the live pointer until the owner accepts
    /// that version.
    pub fn best_skill_markdown(&self) -> String {
        let best = self.validation_best();
        let score = best
            .score
            .map(|score| score.to_string())
            .unwrap_or_else(|| "unscored".to_string());
        let status = if best.accepted && self.live_id == best.id {
            "This version is live."
        } else {
            "This version is the validation winner. It is not live until the owner accepts it."
        };
        format!(
            "# best_skill\n\n{status}\n\n## Instruction\n\n{instruction}\n\n## Validation score\n\n{score}\n",
            instruction = best.instruction,
        )
    }

    pub fn versions(&self) -> &[PromptVersion] {
        &self.versions
    }

    pub fn push_candidate(
        &mut self,
        instruction: impl Into<String>,
        parent_id: Option<u64>,
        score: Option<f64>,
        dimension_scores: Vec<(String, f64)>,
        notes: impl Into<String>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.versions.push(PromptVersion {
            id,
            instruction: instruction.into(),
            parent_id,
            score,
            dimension_scores,
            notes: notes.into(),
            accepted: false,
        });
        id
    }

    pub fn annotate(&mut self, id: u64, notes: impl Into<String>) -> Result<(), PromptOptError> {
        let notes = notes.into();
        let Some(version) = self.versions.iter_mut().find(|version| version.id == id) else {
            return Err(PromptOptError::invalid(format!(
                "unknown prompt version {id}"
            )));
        };
        if !notes.is_empty() {
            if version.notes.is_empty() {
                version.notes = notes;
            } else {
                version.notes = format!("{}\n{notes}", version.notes);
            }
        }
        Ok(())
    }

    /// Owner approval. This is the only way a candidate becomes live.
    pub fn accept(&mut self, id: u64) -> Result<(), PromptOptError> {
        let Some(version) = self.versions.iter_mut().find(|version| version.id == id) else {
            return Err(PromptOptError::invalid(format!(
                "unknown prompt version {id}"
            )));
        };
        version.accepted = true;
        self.live_id = id;
        Ok(())
    }

    /// Move the live pointer back to the baseline or to a version the owner
    /// already accepted. Unaccepted candidates stay in history and do not go live.
    /// Leaving a live version records a rollback signal for the next analyst.
    pub fn rollback(&mut self, id: u64) -> Result<(), PromptOptError> {
        let accepted = match self.get(id) {
            Some(version) => version.accepted,
            None => {
                return Err(PromptOptError::invalid(format!(
                    "unknown prompt version {id}"
                )));
            }
        };
        if id != self.baseline_id && !accepted {
            return Err(PromptOptError::invalid(
                "owner must approve a version before it can go live",
            ));
        }
        let previous = self.live_id;
        self.live_id = id;
        if previous != id {
            self.record_signal(
                EvolutionSignalKind::Rollback,
                previous,
                format!("owner rolled live back from {previous} to {id}"),
            );
        }
        Ok(())
    }

    pub fn signals(&self) -> &[EvolutionSignal] {
        &self.evolution
    }

    pub fn contradictions(&self) -> &[ContradictionItem] {
        &self.contradictions
    }

    pub fn record_signal(
        &mut self,
        kind: EvolutionSignalKind,
        version_id: u64,
        detail: impl Into<String>,
    ) {
        self.evolution.push(EvolutionSignal {
            kind,
            version_id,
            detail: detail.into(),
        });
    }

    /// Owner rejection. The version stays archived. If it was the validation
    /// winner, `best_skill` moves back to its parent or the baseline. Live
    /// text does not change here; use [`Self::rollback`] for that.
    pub fn reject(&mut self, id: u64) -> Result<(), PromptOptError> {
        if self.get(id).is_none() {
            return Err(PromptOptError::invalid(format!(
                "unknown prompt version {id}"
            )));
        }
        if id == self.baseline_id {
            return Err(PromptOptError::invalid("baseline cannot be rejected"));
        }
        if self.best_validation_id == id {
            let parent = self
                .get(id)
                .and_then(|version| version.parent_id)
                .unwrap_or(self.baseline_id);
            self.best_validation_id = if self.get(parent).is_some() {
                parent
            } else {
                self.baseline_id
            };
        }
        self.record_signal(
            EvolutionSignalKind::Reject,
            id,
            "owner rejected this version",
        );
        Ok(())
    }

    /// Record feedback. Conflicting notes are queued for the owner and do not
    /// change the instruction.
    pub fn consider_feedback(&mut self, feedback: &str) -> FeedbackOutcome {
        let instruction = self.live().instruction.clone();
        if feedback_conflicts(feedback) {
            let id = self.next_contradiction_id;
            self.next_contradiction_id += 1;
            self.contradictions.push(ContradictionItem {
                id,
                feedback: feedback.to_string(),
                instruction,
            });
            FeedbackOutcome::ContradictionQueued { id }
        } else {
            FeedbackOutcome::Noted
        }
    }

    pub fn ledger_prompt(&self) -> String {
        if self.evolution.is_empty() {
            return "Negative signals:\n(none)".to_string();
        }
        let lines = self
            .evolution
            .iter()
            .map(|signal| {
                format!(
                    "- {kind:?} version {id}: {detail}",
                    kind = signal.kind,
                    id = signal.version_id,
                    detail = signal.detail,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Negative signals:\n{lines}")
    }
}

fn feedback_conflicts(feedback: &str) -> bool {
    let lower = feedback.to_ascii_lowercase();
    lower.contains("conflict:") || feedback.contains("矛盾") || feedback.contains("冲突")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioning_keeps_baseline_and_rollback_restores_it() {
        let mut registry = PromptRegistry::new("baseline instruction");
        let candidate = registry.push_candidate(
            "candidate instruction",
            Some(1),
            Some(0.8),
            vec![("vitality".to_string(), 0.8)],
            "scored",
        );
        assert_eq!(registry.live().instruction, "baseline instruction");
        assert!(registry.rollback(candidate).is_err());
        assert_eq!(registry.live_id(), registry.baseline_id());
        assert!(registry.promote_if_strictly_better(candidate).unwrap());

        registry.accept(candidate).unwrap();
        assert_eq!(registry.live().instruction, "candidate instruction");
        assert!(registry.live().accepted);

        registry.rollback(registry.baseline_id()).unwrap();
        assert_eq!(registry.live().instruction, "baseline instruction");
        assert_eq!(registry.validation_best_id(), candidate);
        assert!(
            registry
                .best_skill_markdown()
                .contains("candidate instruction")
        );
        assert!(
            registry
                .best_skill_markdown()
                .contains("not live until the owner accepts")
        );
        assert_eq!(registry.versions().len(), 2);
        assert_eq!(
            registry.get(candidate).unwrap().instruction,
            "candidate instruction"
        );
        assert!(registry.rollback(99).is_err());
    }

    #[test]
    fn validation_gate_requires_a_strictly_higher_score() {
        let mut registry = PromptRegistry::new("baseline");
        let weak = registry.push_candidate("weak", Some(1), Some(0.4), Vec::new(), "scored");
        assert!(registry.promote_if_strictly_better(weak).unwrap());
        assert_eq!(registry.validation_best_id(), weak);
        assert_eq!(registry.live_id(), registry.baseline_id());
        assert!(!registry.validation_best().accepted);

        let tied = registry.push_candidate("tied", Some(weak), Some(0.4), Vec::new(), "tied");
        assert!(!registry.promote_if_strictly_better(tied).unwrap());
        assert_eq!(registry.validation_best_id(), weak);

        let worse = registry.push_candidate("worse", Some(weak), Some(0.2), Vec::new(), "worse");
        assert!(!registry.promote_if_strictly_better(worse).unwrap());
        assert_eq!(registry.validation_best().instruction, "weak");

        let better =
            registry.push_candidate("better", Some(weak), Some(0.41), Vec::new(), "better");
        assert!(registry.promote_if_strictly_better(better).unwrap());
        assert!(registry.best_skill_markdown().contains("better"));
        assert_eq!(registry.live().instruction, "baseline");

        let unscored = registry.push_candidate("blank", Some(better), None, Vec::new(), "blank");
        assert!(!registry.promote_if_strictly_better(unscored).unwrap());
        assert_eq!(registry.validation_best_id(), better);
    }

    #[test]
    fn reject_and_rollback_are_negative_signals() {
        let mut registry = PromptRegistry::new("baseline");
        let candidate = registry.push_candidate("next", Some(1), Some(0.8), Vec::new(), "scored");
        assert!(registry.promote_if_strictly_better(candidate).unwrap());
        let before = registry.live().instruction.clone();
        registry.reject(candidate).unwrap();
        assert_eq!(registry.live().instruction, before);
        assert_eq!(registry.validation_best_id(), registry.baseline_id());
        assert!(
            registry
                .versions()
                .iter()
                .any(|version| version.id == candidate)
        );
        assert!(
            registry
                .signals()
                .iter()
                .any(|signal| signal.kind == EvolutionSignalKind::Reject)
        );
        assert!(registry.ledger_prompt().contains("Reject"));

        registry.accept(candidate).unwrap();
        registry.rollback(registry.baseline_id()).unwrap();
        assert_eq!(registry.live().instruction, "baseline");
        assert!(
            registry
                .signals()
                .iter()
                .any(|signal| signal.kind == EvolutionSignalKind::Rollback)
        );
        assert!(registry.reject(registry.baseline_id()).is_err());
    }

    #[test]
    fn conflicting_feedback_is_queued_and_does_not_retune() {
        let mut registry = PromptRegistry::new("one warm scene");
        let noted = registry.consider_feedback("keep the warm light");
        assert_eq!(noted, FeedbackOutcome::Noted);
        assert!(registry.contradictions().is_empty());
        assert_eq!(registry.live().instruction, "one warm scene");

        let queued = registry.consider_feedback("CONFLICT: 不要暖光，这和当前指令矛盾");
        assert_eq!(queued, FeedbackOutcome::ContradictionQueued { id: 1 });
        assert_eq!(registry.contradictions().len(), 1);
        assert_eq!(registry.contradictions()[0].instruction, "one warm scene");
        assert_eq!(registry.live().instruction, "one warm scene");
        assert_eq!(registry.validation_best().instruction, "one warm scene");
    }
}

//! Model capability registry for multimodal routing.
//!
//! OpenRouter exposes model input/output modalities through `/api/v1/models`.
//! This module keeps the runtime decision point capability-driven instead of
//! scattering provider-name string checks through hot paths. Static inference is
//! intentionally conservative and can be overridden from `RuntimeSettings`.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Input/output modality names used by OpenRouter and provider docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modality {
    Text,
    Image,
    Audio,
    File,
    Video,
    Speech,
}

impl Modality {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "text" => Some(Self::Text),
            "image" | "images" | "vision" => Some(Self::Image),
            "audio" | "input_audio" => Some(Self::Audio),
            "file" | "files" | "document" | "pdf" => Some(Self::File),
            "video" | "videos" | "video_url" => Some(Self::Video),
            "speech" | "tts" => Some(Self::Speech),
            _ => None,
        }
    }
}

/// Capability data for one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCapabilities {
    input_modalities: BTreeSet<Modality>,
    output_modalities: BTreeSet<Modality>,
}

impl ModelCapabilities {
    pub fn new(input_modalities: impl IntoIterator<Item = Modality>) -> Self {
        Self {
            input_modalities: input_modalities.into_iter().collect(),
            output_modalities: [Modality::Text].into_iter().collect(),
        }
    }

    pub fn with_outputs(mut self, output_modalities: impl IntoIterator<Item = Modality>) -> Self {
        self.output_modalities = output_modalities.into_iter().collect();
        self
    }

    pub fn supports_input(&self, modality: Modality) -> bool {
        self.input_modalities.contains(&modality)
    }

    pub fn supports_output(&self, modality: Modality) -> bool {
        self.output_modalities.contains(&modality)
    }

    pub fn supports_chat_input_audio_format(&self, format: &str) -> bool {
        self.supports_input(Modality::Audio) && is_chat_input_audio_format_supported(format)
    }

    fn from_json(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let input = obj
            .get("input_modalities")
            .or_else(|| obj.get("inputs"))
            .and_then(parse_modalities)?;
        let output = obj
            .get("output_modalities")
            .or_else(|| obj.get("outputs"))
            .and_then(parse_modalities)
            .unwrap_or_else(|| [Modality::Text].into_iter().collect());

        Some(Self {
            input_modalities: input,
            output_modalities: output,
        })
    }
}

/// Small media plan used by the orchestrator when building content parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaPlan {
    capabilities: ModelCapabilities,
}

impl MediaPlan {
    pub fn for_model(registry: &ModelCapabilityRegistry, model_id: &str) -> Self {
        Self {
            capabilities: registry.capabilities_for(model_id),
        }
    }

    pub fn supports_image(&self) -> bool {
        self.capabilities.supports_input(Modality::Image)
    }

    pub fn supports_file(&self) -> bool {
        self.capabilities.supports_input(Modality::File)
    }

    pub fn supports_video(&self) -> bool {
        self.capabilities.supports_input(Modality::Video)
    }

    pub fn supports_audio_format(&self, format: &str) -> bool {
        self.capabilities.supports_chat_input_audio_format(format)
    }
}

/// Per-call registry. Build from settings so runtime overrides are cheap and
/// require no process restart.
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilityRegistry {
    overrides: HashMap<String, ModelCapabilities>,
}

impl ModelCapabilityRegistry {
    pub fn from_settings(settings: &crate::settings::RuntimeSettings) -> Self {
        let overrides = settings
            .get("model_capability_overrides")
            .and_then(|value| parse_capability_overrides(&value))
            .unwrap_or_default();
        Self { overrides }
    }

    pub fn capabilities_for(&self, model_id: &str) -> ModelCapabilities {
        if let Some(cap) = self.overrides.get(model_id) {
            return cap.clone();
        }
        infer_model_capabilities(model_id)
    }

    pub fn supports_input(&self, model_id: &str, modality: Modality) -> bool {
        self.capabilities_for(model_id).supports_input(modality)
    }
}

/// `async-openai` chat content parts currently serialize native audio only as
/// `mp3` or `wav`. Other formats (Telegram voice is usually OGG/Opus) must go
/// through STT or be omitted with an explicit note; relabeling bytes as MP3 is
/// a provider error.
pub fn is_chat_input_audio_format_supported(format: &str) -> bool {
    matches!(format.to_ascii_lowercase().as_str(), "mp3" | "wav")
}

fn parse_capability_overrides(value: &Value) -> Option<HashMap<String, ModelCapabilities>> {
    let obj = value.as_object()?;
    let mut out = HashMap::new();
    for (model, raw_caps) in obj {
        if let Some(caps) = ModelCapabilities::from_json(raw_caps) {
            out.insert(model.clone(), caps);
        } else {
            tracing::warn!(
                model = %model,
                value = %raw_caps,
                "[model_capabilities] ignoring invalid model_capability_overrides entry"
            );
        }
    }
    Some(out)
}

fn parse_modalities(value: &Value) -> Option<BTreeSet<Modality>> {
    let arr = value.as_array()?;
    let modalities: BTreeSet<Modality> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(Modality::parse)
        .collect();
    if modalities.is_empty() {
        None
    } else {
        Some(modalities)
    }
}

fn infer_model_capabilities(model_id: &str) -> ModelCapabilities {
    let model = model_id.to_ascii_lowercase();
    let mut inputs = BTreeSet::from([Modality::Text]);
    let outputs = BTreeSet::from([Modality::Text]);

    // Unknown model IDs intentionally remain text-only. Operators can supply
    // `model_capability_overrides` when a provider publishes new modalities
    // before this static family inference is updated.
    if model.starts_with("google/gemini") || model.contains("/gemini-") {
        inputs.extend([
            Modality::Image,
            Modality::Audio,
            Modality::File,
            Modality::Video,
        ]);
    } else if model.starts_with("anthropic/claude") {
        inputs.extend([Modality::Image, Modality::File]);
    } else if model.starts_with("openai/gpt-4o")
        || model.starts_with("openai/gpt-5")
        || model.contains("gpt-4o")
        || model.contains("gpt-5")
    {
        inputs.extend([Modality::Image, Modality::Audio, Modality::File]);
    } else if model.starts_with("x-ai/grok") || model.starts_with("xai/grok") {
        inputs.insert(Modality::Image);
    }

    ModelCapabilities {
        input_modalities: inputs,
        output_modalities: outputs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RuntimeSettings;

    #[test]
    fn gemini_infers_all_input_modalities() {
        let caps = infer_model_capabilities("google/gemini-3.1-flash-lite-preview");
        assert!(caps.supports_input(Modality::Image));
        assert!(caps.supports_input(Modality::Audio));
        assert!(caps.supports_input(Modality::File));
        assert!(caps.supports_input(Modality::Video));
    }

    #[test]
    fn grok_does_not_infer_file_or_audio() {
        let caps = infer_model_capabilities("x-ai/grok-4.1-fast");
        assert!(caps.supports_input(Modality::Image));
        assert!(!caps.supports_input(Modality::Audio));
        assert!(!caps.supports_input(Modality::File));
        assert!(!caps.supports_input(Modality::Video));
    }

    #[test]
    fn settings_override_exact_model_capabilities() {
        let settings = RuntimeSettings::new();
        settings
            .set(
                "model_capability_overrides",
                serde_json::json!({
                    "vendor/custom": {
                        "input_modalities": ["text", "video"],
                        "output_modalities": ["text"]
                    }
                }),
            )
            .unwrap();
        let registry = ModelCapabilityRegistry::from_settings(&settings);
        assert!(registry.supports_input("vendor/custom", Modality::Video));
        assert!(!registry.supports_input("vendor/custom", Modality::Image));
    }

    #[test]
    fn chat_audio_accepts_only_mp3_and_wav() {
        assert!(is_chat_input_audio_format_supported("mp3"));
        assert!(is_chat_input_audio_format_supported("WAV"));
        assert!(!is_chat_input_audio_format_supported("ogg"));
        assert!(!is_chat_input_audio_format_supported("m4a"));
    }
}

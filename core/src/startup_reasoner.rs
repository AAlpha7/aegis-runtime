//! Startup gate: grok build, Codex, or an LLM API key — one of three required.
//!
//! A fresh install must fail here, naming every accepted option, instead of
//! dying later on the first chat with a provider 401. Isolated-boot dummy
//! keys and official CLI install locations still count.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Values `.env.example` ships for `OPENAI_API_KEY` (plus the historical
/// `sk-placeholder`). Exact strings only: dry-run keys such as `sk-dryrun`
/// or the isolated-boot dummy key must keep starting the service.
pub const OPENAI_API_KEY_PLACEHOLDERS: &[&str] = &["sk-placeholder", "sk-or-v1-xxxxxxxx"];

/// Value `.env.example` ships (commented out) for `EMBEDDING_API_KEY`.
pub const EMBEDDING_API_KEY_PLACEHOLDERS: &[&str] = &["sk-xxxxxxxx"];

/// Official Grok Build installer location (`curl -fsSL https://x.ai/cli/install.sh`).
pub const GROK_OFFICIAL_RELATIVE: &str = ".grok/bin/grok";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupReasoner {
    ApiKey,
    GrokBuild,
    Codex,
}

impl StartupReasoner {
    pub fn label(self) -> &'static str {
        match self {
            Self::ApiKey => "API key (OPENAI_API_KEY)",
            Self::GrokBuild => "grok build CLI",
            Self::Codex => "Codex CLI",
        }
    }

    /// True when this path is a coding-agent CLI rather than a paid API key.
    pub fn is_external_executor(self) -> bool {
        matches!(self, Self::GrokBuild | Self::Codex)
    }
}

/// Inputs for the triad check. Every field is injectable so tests never
/// depend on the host `PATH` or a real `.env`.
#[derive(Debug, Clone)]
pub struct StartupReasonerInputs<'a> {
    pub openai_api_key: Option<&'a str>,
    pub grok_bin: &'a str,
    pub codex_bin: &'a str,
    pub grok_well_known: Option<&'a Path>,
}

struct FromEnvOwned {
    openai_api_key: Option<String>,
    grok_bin: String,
    codex_bin: String,
    grok_well_known: Option<PathBuf>,
}

impl FromEnvOwned {
    fn capture() -> Self {
        let grok_bin = std::env::var("AEGIS_EXECUTOR_GROK_BIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "grok".to_string());
        let codex_bin = std::env::var("AEGIS_EXECUTOR_CODEX_BIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "codex".to_string());
        Self {
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            grok_bin,
            codex_bin,
            grok_well_known: official_grok_bin(),
        }
    }

    fn as_inputs(&self) -> StartupReasonerInputs<'_> {
        StartupReasonerInputs {
            openai_api_key: self.openai_api_key.as_deref(),
            grok_bin: &self.grok_bin,
            codex_bin: &self.codex_bin,
            grok_well_known: self.grok_well_known.as_deref(),
        }
    }
}

/// `$HOME/.grok/bin/grok` when `HOME` is set.
pub fn official_grok_bin() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(GROK_OFFICIAL_RELATIVE))
}

/// Pure check for `OPENAI_API_KEY` (`None` = unset). Rejects unset,
/// empty/whitespace, and the exact template placeholders.
pub fn validate_llm_api_key(value: Option<&str>) -> Result<()> {
    match value {
        None => anyhow::bail!(
            "OPENAI_API_KEY not found in environment. \
             Copy .env.example to .env and configure your LLM API key, \
             or install grok build / Codex (one of the three is required)."
        ),
        Some(key) if key.trim().is_empty() => anyhow::bail!(
            "OPENAI_API_KEY is empty. Set a valid API key in .env \
             (OpenRouter: sk-or-v1-... or Ollama: any non-empty value)"
        ),
        Some(key) if OPENAI_API_KEY_PLACEHOLDERS.contains(&key.trim()) => anyhow::bail!(
            "OPENAI_API_KEY is still the template placeholder `{}` copied from .env.example. \
             Edit .env (or the EnvironmentFile your service loads) and replace it with your \
             real key (OpenRouter: sk-or-v1-... or Ollama: any non-empty value).",
            key.trim()
        ),
        Some(_) => Ok(()),
    }
}

/// `EMBEDDING_API_KEY` is optional (defaults to `OPENAI_API_KEY`). Only the
/// exact template placeholder is rejected.
pub fn validate_embedding_api_key(value: Option<&str>) -> Result<()> {
    match value {
        Some(key) if EMBEDDING_API_KEY_PLACEHOLDERS.contains(&key.trim()) => anyhow::bail!(
            "EMBEDDING_API_KEY is still the template placeholder `{}` copied from .env.example. \
             Edit .env (or the EnvironmentFile your service loads): set the real embeddings \
             key, or remove the line so it defaults to OPENAI_API_KEY.",
            key.trim()
        ),
        _ => Ok(()),
    }
}

pub fn llm_api_key_usable(value: Option<&str>) -> bool {
    validate_llm_api_key(value).is_ok()
}

/// Host `PATH` / absolute-path probe used in production.
pub fn host_binary_available(program: &str) -> bool {
    binary_available(program, Path::is_file)
}

fn binary_available(program: &str, exists: impl Fn(&Path) -> bool) -> bool {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') {
        return exists(path);
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| exists(&dir.join(program))))
        .unwrap_or(false)
}

/// Resolve which of the three required reasoners is present.
///
/// Preference when several are present: a usable `OPENAI_API_KEY` wins, then
/// grok build, then Codex. Isolated boot keeps working because it sets a
/// dummy key.
pub fn validate_startup_reasoner(
    inputs: &StartupReasonerInputs<'_>,
    which: impl Fn(&str) -> bool,
) -> Result<StartupReasoner> {
    if llm_api_key_usable(inputs.openai_api_key) {
        return Ok(StartupReasoner::ApiKey);
    }

    let grok_ok = which(inputs.grok_bin)
        || inputs
            .grok_well_known
            .map(|path| which(&path.to_string_lossy()))
            .unwrap_or(false);
    if grok_ok {
        return Ok(StartupReasoner::GrokBuild);
    }

    if which(inputs.codex_bin) {
        return Ok(StartupReasoner::Codex);
    }

    let mut msg = String::from(
        "Aegis Runtime needs one of: grok build, Codex, or an LLM API key.\n\
         \n\
           • grok build — install the CLI (`curl -fsSL https://x.ai/cli/install.sh | bash`) \
         so `grok` is on PATH (or at ~/.grok/bin/grok)\n\
           • Codex — install the Codex CLI so `codex` is on PATH\n\
           • API key — set OPENAI_API_KEY in .env (OpenRouter: sk-or-v1-... or \
         Ollama: any non-empty value)\n\
         \n\
         None of these were found.",
    );
    if let Err(key_err) = validate_llm_api_key(inputs.openai_api_key) {
        let key_msg = key_err.to_string();
        if key_msg.contains("placeholder") || key_msg.contains("empty") {
            msg.push_str("\n\nOPENAI_API_KEY detail: ");
            msg.push_str(&key_msg);
        }
    }
    anyhow::bail!("{msg}");
}

/// Live-environment triad check used by the binary at boot.
pub fn resolve_startup_reasoner_from_env() -> Result<StartupReasoner> {
    let owned = FromEnvOwned::capture();
    let inputs = owned.as_inputs();
    validate_startup_reasoner(&inputs, host_binary_available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn inputs<'a>(
        key: Option<&'a str>,
        grok: &'a str,
        codex: &'a str,
        well_known: Option<&'a Path>,
    ) -> StartupReasonerInputs<'a> {
        StartupReasonerInputs {
            openai_api_key: key,
            grok_bin: grok,
            codex_bin: codex,
            grok_well_known: well_known,
        }
    }

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn rejects_nothing_installed() {
        let err = validate_startup_reasoner(&inputs(None, "grok", "codex", None), none)
            .expect_err("no reasoner must fail");
        let msg = err.to_string();
        assert!(msg.contains("grok build"), "{msg}");
        assert!(msg.contains("Codex"), "{msg}");
        assert!(msg.contains("OPENAI_API_KEY"), "{msg}");
        assert!(msg.contains("https://x.ai/cli/install.sh"), "{msg}");
    }

    #[test]
    fn rejects_template_placeholder_when_no_cli() {
        let err = validate_startup_reasoner(
            &inputs(Some("sk-or-v1-xxxxxxxx"), "grok", "codex", None),
            none,
        )
        .expect_err("placeholder without CLI must fail");
        let msg = err.to_string();
        assert!(msg.contains("placeholder"), "{msg}");
        assert!(msg.contains("sk-or-v1-xxxxxxxx"), "{msg}");
        assert!(msg.contains("grok build"), "{msg}");
    }

    #[test]
    fn accepts_usable_api_key_without_cli() {
        let reasoner = validate_startup_reasoner(
            &inputs(
                Some("sk-or-v1-hercules-isolated-dummy-not-real"),
                "grok",
                "codex",
                None,
            ),
            none,
        )
        .expect("dummy isolated key is a usable API key");
        assert_eq!(reasoner, StartupReasoner::ApiKey);
    }

    #[test]
    fn accepts_grok_on_path_without_api_key() {
        let reasoner =
            validate_startup_reasoner(&inputs(None, "grok", "codex", None), |p| p == "grok")
                .expect("grok CLI is enough");
        assert_eq!(reasoner, StartupReasoner::GrokBuild);
        assert!(reasoner.is_external_executor());
    }

    #[test]
    fn accepts_official_grok_install_path_without_path_entry() {
        let well = PathBuf::from("/home/ubuntu/.grok/bin/grok");
        let reasoner =
            validate_startup_reasoner(&inputs(None, "grok", "codex", Some(&well)), |p| {
                p == "/home/ubuntu/.grok/bin/grok"
            })
            .expect("official installer location counts");
        assert_eq!(reasoner, StartupReasoner::GrokBuild);
    }

    #[test]
    fn accepts_codex_on_path_without_api_key() {
        let reasoner =
            validate_startup_reasoner(&inputs(None, "grok", "codex", None), |p| p == "codex")
                .expect("codex CLI is enough");
        assert_eq!(reasoner, StartupReasoner::Codex);
    }

    #[test]
    fn placeholder_plus_grok_is_grok_not_key() {
        let reasoner = validate_startup_reasoner(
            &inputs(Some("sk-or-v1-xxxxxxxx"), "grok", "codex", None),
            |p| p == "grok",
        )
        .expect("placeholder must not block grok");
        assert_eq!(reasoner, StartupReasoner::GrokBuild);
    }

    #[test]
    fn empty_key_plus_grok_is_grok() {
        let reasoner =
            validate_startup_reasoner(&inputs(Some(""), "grok", "codex", None), |p| p == "grok")
                .expect("empty key must not block grok");
        assert_eq!(reasoner, StartupReasoner::GrokBuild);
    }

    #[test]
    fn usable_key_wins_over_cli() {
        let reasoner =
            validate_startup_reasoner(&inputs(Some("sk-dryrun"), "grok", "codex", None), |p| {
                p == "grok" || p == "codex"
            })
            .expect("usable key is preferred");
        assert_eq!(reasoner, StartupReasoner::ApiKey);
        assert!(!reasoner.is_external_executor());
    }

    #[test]
    fn grok_wins_over_codex_when_no_key() {
        let reasoner = validate_startup_reasoner(&inputs(None, "grok", "codex", None), |p| {
            p == "grok" || p == "codex"
        })
        .expect("grok preferred over codex");
        assert_eq!(reasoner, StartupReasoner::GrokBuild);
    }

    #[test]
    fn llm_key_rejects_template_placeholders_naming_variable_and_file() {
        for placeholder in ["sk-or-v1-xxxxxxxx", "sk-placeholder", " sk-or-v1-xxxxxxxx "] {
            let err = validate_llm_api_key(Some(placeholder))
                .expect_err("template placeholder must be rejected");
            let msg = err.to_string();
            assert!(msg.contains("OPENAI_API_KEY"), "names the variable: {msg}");
            assert!(msg.contains(".env.example"), "names the template: {msg}");
            assert!(msg.contains(".env"), "names the file to edit: {msg}");
            assert!(msg.contains(placeholder.trim()), "echoes the value: {msg}");
        }
    }

    #[test]
    fn llm_key_rejects_unset_and_empty() {
        let unset = validate_llm_api_key(None).expect_err("unset key must fail");
        assert!(unset.to_string().contains("Copy .env.example to .env"));
        for empty in ["", "   "] {
            let err = validate_llm_api_key(Some(empty)).expect_err("empty key must fail");
            assert!(err.to_string().contains("OPENAI_API_KEY is empty"));
        }
    }

    #[test]
    fn llm_key_accepts_real_looking_dry_run_and_near_miss_values() {
        for key in [
            "sk-or-v1-0123456789abcdef0123456789abcdef",
            "sk-dryrun",
            "ollama",
            "sk-or-v1-hercules-isolated-dummy-not-real",
            "sk-or-v1-xxxxxxxxx",
            "sk-or-v1-xxxxxxx",
        ] {
            validate_llm_api_key(Some(key))
                .unwrap_or_else(|e| panic!("{key:?} must be accepted, got: {e}"));
        }
    }

    #[test]
    fn embedding_key_rejects_only_the_template_placeholder() {
        let err = validate_embedding_api_key(Some("sk-xxxxxxxx"))
            .expect_err("template placeholder must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("EMBEDDING_API_KEY") && msg.contains(".env.example"));
        for value in [
            None,
            Some("sk-real-embeddings-key"),
            Some("sk-xxxxxxx"),
            Some(""),
        ] {
            validate_embedding_api_key(value)
                .unwrap_or_else(|e| panic!("{value:?} must be accepted, got: {e}"));
        }
    }

    #[test]
    fn binary_available_accepts_absolute_path_via_exists() {
        assert!(binary_available("/opt/grok", |_| true));
        assert!(!binary_available("/opt/grok", |_| false));
        assert!(binary_available("./rel/grok", |_| true));
    }
}

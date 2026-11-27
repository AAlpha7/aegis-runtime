// ── Crate-level lint configuration ─────────────────────────────────────────
// Structural choices: many orchestrator helpers genuinely need 8+ params and
// complex callback types. Suppressing these avoids churn without hiding bugs.
#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::new_without_default)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::result_unit_err)]
#![allow(clippy::manual_async_fn)]
#![allow(clippy::should_implement_trait)]
#![allow(clippy::assertions_on_constants)] // compile-time constant validation in tests

// ── Shared macros (must precede modules that use them) ─────────────────────
#[macro_use]
mod macros;

// ── Active modules ──────────────────────────────────────────────────────────
pub mod action_ledger;
pub mod admin;
pub mod agent_skills;
pub mod alert_copy;
pub mod alert_gate;
pub mod assistant_events;
pub mod assistant_work_items;
mod callback_routes;
pub mod outbound_privacy;
// Test-support re-export: the S1 routing-linkage sensor
// (tests/hitl_routing_linkage.rs) iterates the canonical callback registry.
#[doc(hidden)]
pub use callback_routes::callback_route_matrix;
pub mod behavior_notice;
pub mod call_validation;
pub mod ceo_intent;
pub mod chat_stream;
pub mod chronos;
mod chronos_analysis;
mod chronos_heartbeat;
mod chronos_probes;
pub(crate) mod compaction;
pub mod computer_use;
pub mod credential_store;
pub mod daily_brief;
pub mod data_access_gate;
pub mod egress_gate;
pub mod embedding;
pub mod embedding_corpus;
pub mod encrypted_sqlite;
pub mod evaluators;
pub mod evidence;
pub mod executor_client;
pub mod external_executor;
pub mod google_auth;
pub mod heartbeat_config;
pub mod heirloom;
pub mod hercules_client;
pub mod hitl;
pub mod hnsw_index;
pub mod i18n;
pub mod identity;
pub mod intent;
pub mod jev_config;
pub mod llm;
pub mod llm_client;
pub mod llm_pricing;
pub mod logging;
pub mod mcp_client;
pub mod memory;
pub mod memory_heirloom_bridge;
pub(crate) mod memory_migrations;
pub mod menu;
pub mod model_capabilities;
pub mod model_catalog;
pub mod models;
pub mod multimodal;
pub mod openrouter;
pub mod orchestrator;
mod orchestrator_panels;
mod orchestrator_tools_browser;
mod orchestrator_tools_calendar;
mod orchestrator_tools_computer_use;
mod orchestrator_tools_delegation;
pub mod orchestrator_tools_external;
mod orchestrator_tools_goals;
mod orchestrator_tools_knowledge;
mod orchestrator_tools_skills;
mod orchestrator_tools_subcall;
mod orchestrator_tools_system;
mod orchestrator_tools_tasks;
mod orchestrator_tools_ui;
mod orchestrator_tools_web;
pub mod owner_presence;
pub mod profile_registry;
pub mod progress_router;
pub mod prompt_opt;
pub mod prompts;
pub mod recall;
pub mod sandbox;
pub(crate) mod sandbox_host_fns;
pub(crate) mod sandbox_network;
pub(crate) mod sandbox_v2;
pub mod search_turn;
pub mod security;
pub mod settings;
pub mod settings_audit;
pub mod skill_opt;
pub mod snapshot;
pub(crate) mod source_guards;
pub mod startup_reasoner;
pub mod subcall;
pub mod subcall_concurrency;
mod subcall_recall;
pub mod telegram;
pub mod telegram_health;
pub mod telegram_notification_router;
pub mod telegram_render;
pub mod tool_gate;
pub mod tool_schemas;
pub mod topic_router;
pub mod trigger;
pub mod user_mandates;
pub mod utils;
pub mod voice_session;
pub mod web_console_auth;
pub mod web_search;
pub(crate) mod webapp_assets;
pub mod webapp_auth;
pub mod wit_bindings;
pub mod wit_contract;

pub mod health_analyzer;
pub mod self_audit;

// ── Shared test helpers for orchestrator_tools_* unit tests ────────────────
#[cfg(test)]
pub(crate) mod test_tool_helpers {
    use std::sync::Arc;

    use async_openai::types::chat::{
        ChatCompletionMessageToolCall, ChatCompletionRequestMessage, FunctionCall,
    };

    use crate::embedding::EmbeddingClient;
    use crate::identity::AgentIdentity;
    use crate::memory::MemoryHandle;

    use crate::orchestrator::{NotifyUserRequest, Orchestrator, ToolContext};
    use crate::sandbox::SandboxEngine;
    use crate::settings::RuntimeSettings;

    pub fn make_tool_call(name: &str, args: &str) -> ChatCompletionMessageToolCall {
        ChatCompletionMessageToolCall {
            id: format!("call_{name}"),
            function: FunctionCall {
                name: name.to_string(),
                arguments: args.to_string(),
            },
        }
    }

    /// Build a minimal Orchestrator suitable for testing tool handlers that
    /// don't require an actual LLM or network services.
    pub fn test_orchestrator(memory: MemoryHandle, settings: Arc<RuntimeSettings>) -> Orchestrator {
        let sandbox = Arc::new(SandboxEngine::new().expect("sandbox engine"));
        let identity = Arc::new(AgentIdentity::generate());
        let embedder = EmbeddingClient::new("http://127.0.0.1:1/v1", "test-key", "test-model", 384);
        Orchestrator::new(
            "test-key",
            "http://127.0.0.1:1/v1",
            "test-model",
            sandbox,
            identity,
            memory,
            embedder,
            settings,
        )
    }

    /// Mutable state bucket for constructing `ToolContext` in tests.
    pub struct CtxState {
        pub prev_results: Vec<String>,
        pub tools_called: Vec<String>,
        pub tools_blocked: Vec<String>,
        pub messages: Vec<ChatCompletionRequestMessage>,
        pub total_calls: usize,
        pub notifications: Vec<NotifyUserRequest>,
        pub promoted_tools: Vec<String>,
    }

    impl CtxState {
        pub fn new() -> Self {
            Self {
                prev_results: Vec::new(),
                tools_called: Vec::new(),
                tools_blocked: Vec::new(),
                messages: Vec::new(),
                total_calls: 0,
                notifications: Vec::new(),
                promoted_tools: Vec::new(),
            }
        }

        pub fn ctx(&mut self) -> ToolContext<'_> {
            self.ctx_with_chat_id(12345)
        }

        pub fn ctx_with_chat_id(&mut self, chat_id: i64) -> ToolContext<'_> {
            self.ctx_with_caller(chat_id, crate::tool_gate::CallerContext::Interactive)
        }

        /// Variant that lets tests pin the `caller_context` (used by
        /// `orchestrator_tools_subcall` tests to exercise the Subcall
        /// nesting guard).
        pub fn ctx_with_caller(
            &mut self,
            chat_id: i64,
            caller_context: crate::tool_gate::CallerContext,
        ) -> ToolContext<'_> {
            ToolContext {
                user_did: "did:test:user",
                chat_id: Some(chat_id),
                loaded_skills: &[],
                previous_tool_results: &mut self.prev_results,
                tools_called: &mut self.tools_called,
                tools_blocked: &mut self.tools_blocked,
                messages: &mut self.messages,
                total_tool_call_count: &mut self.total_calls,
                max_tool_calls: 35,
                is_callback: false,
                callback_depth: 0,
                notifications: &mut self.notifications,
                promoted_tools: &mut self.promoted_tools,
                autonomous_config: None,
                data_access_context: None,
                heartbeat_tick_state: None,
                caller_context,
                parent_token_budget: (128_000, 128_000),
                gate_pre_approved: false,
                search_turn: None,
            }
        }
    }
}
pub mod hot_reload;
pub mod mcp_server;
pub mod remote_access;
pub mod ux_reviewer;

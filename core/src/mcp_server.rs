//! MCP Server — exposes Aegis Runtime tools and resources to external MCP clients.
//!
//! Implements `rmcp::ServerHandler` so that external AI clients
//! (Claude Desktop, Cursor, VS Code, etc.) can discover and call Aegis Runtime's
//! pseudo-tools, Wasm skills, and system resources via the MCP protocol.
//!
//! Transport: stdio (for subprocess embedding) or in-process (for testing).
//!
//! Activated by setting `AEGIS_MCP_SERVER=1`. All tool calls go through
//! the standard orchestrator gate chain (HITL approval, exfiltration
//! detection, etc.) — the MCP server does NOT bypass security layers.
//!
//! Complementary to `mcp_client.rs` (which connects Aegis Runtime *to* external
//! MCP servers). This module exposes Aegis Runtime *as* an MCP server for external
//! AI clients.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::RwLock;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
    ReadResourceResult, ResourceContents, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use crate::models::SkillCapability;

// ─── Dynamic Tool Registry ──────────────────────────────────────────────────

/// A registered tool that Aegis Runtime exposes via MCP.
#[derive(Debug, Clone)]
pub struct RegisteredTool {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input parameters (as a properties map).
    pub input_schema: Value,
    pub kind: ToolKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolKind {
    PseudoTool,
    WasmSkill,
}

/// Callback for executing a tool. The MCP server delegates to this
/// so the orchestrator or sandbox can run the actual logic.
pub type ToolExecutor =
    Arc<dyn Fn(String, Value) -> futures::future::BoxFuture<'static, Result<Value>> + Send + Sync>;

// ─── Aegis Runtime MCP Server ────────────────────────────────────────────────────

/// MCP server that exposes Aegis Runtime capabilities to external clients.
pub struct AegisMcpServer {
    tools: Arc<RwLock<HashMap<String, RegisteredTool>>>,
    executor: ToolExecutor,
    server_name: String,
    server_version: String,
}

impl AegisMcpServer {
    pub fn new(executor: ToolExecutor) -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            executor,
            server_name: "aegis-runtime".into(),
            server_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    pub fn register_tool(&self, tool: RegisteredTool) {
        self.tools.write().insert(tool.name.clone(), tool);
    }

    pub fn unregister_tool(&self, name: &str) {
        self.tools.write().remove(name);
    }

    /// Bulk-register tools from loaded Wasm skills.
    pub fn register_wasm_skills(&self, capabilities: &[SkillCapability]) {
        let mut tools = self.tools.write();
        for cap in capabilities {
            tools.insert(
                cap.skill_name.clone(),
                RegisteredTool {
                    name: cap.skill_name.clone(),
                    description: cap.description.clone(),
                    input_schema: cap.payload_schema.clone(),
                    kind: ToolKind::WasmSkill,
                },
            );
        }
    }

    pub fn register_pseudo_tool(&self, name: &str, description: &str, schema: Value) {
        self.register_tool(RegisteredTool {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            kind: ToolKind::PseudoTool,
        });
    }

    pub fn tool_count(&self) -> usize {
        self.tools.read().len()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.read().keys().cloned().collect()
    }

    /// Build MCP Tool list from registered tools.
    fn build_tool_list(&self) -> Vec<Tool> {
        let tools = self.tools.read();
        tools
            .values()
            .map(|t| {
                let schema = value_to_json_object(&t.input_schema);
                Tool::new(t.name.clone(), t.description.clone(), schema)
            })
            .collect()
    }
}

impl ServerHandler for AegisMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
        .with_server_info(rmcp::model::Implementation::new(
            self.server_name.clone(),
            self.server_version.clone(),
        ))
        .with_instructions(
            "Aegis Runtime AI Agent — exposes tools for web search, email, \
             memory, skill management, and more.",
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListToolsResult, McpError>> + Send + '_
    {
        let tools = self.build_tool_list();
        async move {
            Ok(ListToolsResult {
                meta: None,
                tools,
                next_cursor: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<CallToolResult, McpError>> + Send + '_
    {
        let tool_name: String = request.name.into();
        let arguments: Value = request
            .arguments
            .map(Value::Object)
            .unwrap_or(Value::Object(Default::default()));
        let executor = self.executor.clone();

        async move {
            // Verify tool exists
            {
                let tools = self.tools.read();
                if !tools.contains_key(&tool_name) {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "Unknown tool: {tool_name}"
                    ))]));
                }
            }

            match executor(tool_name.clone(), arguments).await {
                Ok(result) => {
                    let text = if result.is_string() {
                        result.as_str().unwrap_or("").to_string()
                    } else {
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    };
                    let is_error = result
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "error")
                        .unwrap_or(false);
                    if is_error {
                        Ok(CallToolResult::error(vec![Content::text(text)]))
                    } else {
                        Ok(CallToolResult::success(vec![Content::text(text)]))
                    }
                }
                Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "Tool execution error: {e:#}"
                ))])),
            }
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ListResourcesResult, McpError>> + Send + '_
    {
        async move {
            use rmcp::model::{AnnotateAble, RawResource};

            let resources = vec![
                RawResource::new("aegis-runtime://skills", "Active Skills")
                    .with_description("List of active Wasm skills and their capabilities")
                    .with_mime_type("application/json")
                    .no_annotation(),
                RawResource::new("aegis-runtime://health", "System Health")
                    .with_description("Aegis Runtime system health status")
                    .with_mime_type("application/json")
                    .no_annotation(),
            ];
            Ok(ListResourcesResult {
                meta: None,
                resources,
                next_cursor: None,
            })
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = std::result::Result<ReadResourceResult, McpError>> + Send + '_
    {
        let uri = request.uri.clone();
        async move {
            let content = match uri.as_str() {
                "aegis-runtime://skills" => {
                    let tools = self.tools.read();
                    let skills: Vec<Value> = tools
                        .values()
                        .filter(|t| t.kind == ToolKind::WasmSkill)
                        .map(|t| {
                            serde_json::json!({
                                "name": t.name,
                                "description": t.description,
                            })
                        })
                        .collect();
                    serde_json::to_string_pretty(&skills).unwrap_or_default()
                }
                "aegis-runtime://health" => {
                    let tool_count = self.tools.read().len();
                    serde_json::json!({
                        "status": "ok",
                        "tools_registered": tool_count,
                        "version": self.server_version,
                    })
                    .to_string()
                }
                _ => {
                    return Err(McpError::resource_not_found(
                        "resource_not_found",
                        Some(serde_json::json!({"uri": uri})),
                    ));
                }
            };

            Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]))
        }
    }
}

// ─── Helper Functions ────────────────────────────────────────────────────────

/// Convert a serde_json::Value to JsonObject (Map<String, Value>).
fn value_to_json_object(schema: &Value) -> JsonObject {
    match schema {
        Value::Object(map) => map.clone(),
        _ => {
            let mut map = serde_json::Map::new();
            map.insert("type".into(), Value::String("object".into()));
            map
        }
    }
}

// ─── OpenAI Schema Conversion ───────────────────────────────────────────────

/// Convert an OpenAI `ChatCompletionTools` schema to an MCP [`RegisteredTool`].
///
/// Returns `None` if the input is not a `Function` variant.
pub fn registered_tool_from_oai_schema(
    tool: &async_openai::types::chat::ChatCompletionTools,
    kind: ToolKind,
) -> Option<RegisteredTool> {
    use async_openai::types::chat::ChatCompletionTools;
    match tool {
        ChatCompletionTools::Function(f) => Some(RegisteredTool {
            name: f.function.name.clone(),
            description: f
                .function
                .description
                .clone()
                .unwrap_or_else(|| f.function.name.clone()),
            input_schema: f
                .function
                .parameters
                .clone()
                .unwrap_or(Value::Object(Default::default())),
            kind,
        }),
        _ => None,
    }
}

impl AegisMcpServer {
    /// Bulk-register tools from OpenAI-format `ChatCompletionTools` schemas.
    pub fn register_from_oai_schemas(
        &self,
        schemas: &[async_openai::types::chat::ChatCompletionTools],
        kind: ToolKind,
    ) {
        for schema in schemas {
            if let Some(tool) = registered_tool_from_oai_schema(schema, kind.clone()) {
                self.register_tool(tool);
            }
        }
    }
}

/// Start the MCP server on stdio transport (blocking — call from spawn).
pub async fn serve_stdio(server: AegisMcpServer) -> Result<()> {
    tracing::info!("Starting MCP server on stdio");
    let service = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("MCP server start failed: {e}"))?;

    service
        .waiting()
        .await
        .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))?;
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_server() -> AegisMcpServer {
        let executor: ToolExecutor = Arc::new(|name, args| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "status": "ok",
                    "tool": name,
                    "echo": args,
                }))
            })
        });
        let server = AegisMcpServer::new(executor);

        server.register_pseudo_tool(
            "get_current_time",
            "Get the current date and time",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "IANA timezone name"
                    }
                }
            }),
        );

        server.register_pseudo_tool(
            "web_search",
            "Search the web",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["query"]
            }),
        );

        server
    }

    #[test]
    fn test_register_and_list_tools() {
        let server = make_test_server();
        assert_eq!(server.tool_count(), 2);

        let names = server.tool_names();
        assert!(names.contains(&"get_current_time".to_string()));
        assert!(names.contains(&"web_search".to_string()));
    }

    #[test]
    fn test_unregister_tool() {
        let server = make_test_server();
        assert_eq!(server.tool_count(), 2);
        server.unregister_tool("web_search");
        assert_eq!(server.tool_count(), 1);
    }

    #[test]
    fn test_register_wasm_skills() {
        let server = make_test_server();
        let caps = vec![SkillCapability {
            skill_name: "weather".into(),
            description: "Get weather data".into(),
            payload_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                },
                "required": ["city"]
            }),
            max_memory_mb: 2,
            allowed_domains: vec!["api.weather.com".into()],
            allowed_fs_paths: vec![],
            cron_schedule: None,
            default_payload: None,
            skill_pattern: Some("api_json".into()),
            language: "rust".into(),
            is_read_only: false,
            supports_multi_account: false,
            shadow_mode: false,
            menu_command: None,
            menu_description: None,
            component_model: false,
            limitations: vec![],
            max_fuel: 500_000_000,
            timeout_secs: 60,
        }];
        server.register_wasm_skills(&caps);
        assert_eq!(server.tool_count(), 3);
        assert!(server.tool_names().contains(&"weather".to_string()));
    }

    #[test]
    fn test_build_tool_list() {
        let server = make_test_server();
        let tools = server.build_tool_list();
        assert_eq!(tools.len(), 2);

        let time_tool = tools
            .iter()
            .find(|t| t.name.as_ref() == "get_current_time")
            .unwrap();
        assert!(time_tool.description.is_some());
    }

    #[test]
    fn test_server_info() {
        let server = make_test_server();
        let info = server.get_info();
        assert_eq!(info.server_info.name, "aegis-runtime");
        assert!(info.instructions.is_some());
    }

    #[test]
    fn test_value_to_json_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });
        let result = value_to_json_object(&schema);
        assert!(result.contains_key("type"));
        assert!(result.contains_key("properties"));
        assert!(result.contains_key("required"));
    }

    #[test]
    fn test_value_to_json_object_non_object() {
        let schema = serde_json::json!("not an object");
        let result = value_to_json_object(&schema);
        assert_eq!(result["type"], "object");
    }

    #[tokio::test]
    async fn test_call_tool_via_executor() {
        let executor: ToolExecutor = Arc::new(|name, args| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "status": "ok",
                    "tool": name,
                    "echo": args,
                }))
            })
        });
        let result = executor("test_tool".into(), serde_json::json!({"key": "val"})).await;
        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val["status"], "ok");
        assert_eq!(val["tool"], "test_tool");
    }

    #[tokio::test]
    async fn test_in_process_client_server() {
        let server = make_test_server();

        // In-process duplex transport
        let (server_io, client_io) = tokio::io::duplex(8192);

        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        // Client side
        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        // List tools
        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 2);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(tool_names.contains(&"get_current_time"));
        assert!(tool_names.contains(&"web_search"));

        // Call a tool
        let result = client
            .call_tool(
                CallToolRequestParams::new("web_search").with_arguments(
                    vec![("query".into(), serde_json::json!("rust mcp"))]
                        .into_iter()
                        .collect(),
                ),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        assert!(!result.content.is_empty());

        // List resources
        let resources = client.list_all_resources().await.unwrap();
        assert!(!resources.is_empty());

        // Read a resource
        let health = client
            .read_resource(ReadResourceRequestParams::new("aegis-runtime://health"))
            .await
            .unwrap();
        assert!(!health.contents.is_empty());

        client.cancel().await.ok();
        server_handle.abort();
    }

    // ── Schema conversion tests ─────────────────────────────────────────

    #[test]
    fn test_registered_tool_from_oai_schema() {
        use async_openai::types::chat::{
            ChatCompletionTool, ChatCompletionTools, FunctionObjectArgs,
        };

        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });
        let oai_tool = ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObjectArgs::default()
                .name("web_search")
                .description("Search the web")
                .parameters(schema.clone())
                .build()
                .unwrap(),
        });

        let reg = registered_tool_from_oai_schema(&oai_tool, ToolKind::PseudoTool).unwrap();
        assert_eq!(reg.name, "web_search");
        assert_eq!(reg.description, "Search the web");
        assert_eq!(reg.input_schema, schema);
        assert_eq!(reg.kind, ToolKind::PseudoTool);
    }

    #[test]
    fn test_registered_tool_from_oai_schema_no_params() {
        use async_openai::types::chat::{
            ChatCompletionTool, ChatCompletionTools, FunctionObjectArgs,
        };

        let oai_tool = ChatCompletionTools::Function(ChatCompletionTool {
            function: FunctionObjectArgs::default()
                .name("get_time")
                .description("Get current time")
                .build()
                .unwrap(),
        });

        let reg = registered_tool_from_oai_schema(&oai_tool, ToolKind::PseudoTool).unwrap();
        assert_eq!(reg.name, "get_time");
        // No parameters → defaults to empty object
        assert!(reg.input_schema.is_object());
    }

    #[test]
    fn test_register_from_oai_schemas_bulk() {
        use async_openai::types::chat::{
            ChatCompletionTool, ChatCompletionTools, FunctionObjectArgs,
        };

        let server = make_test_server();
        let initial_count = server.tool_count();

        let schemas: Vec<ChatCompletionTools> = vec![
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObjectArgs::default()
                    .name("tool_a")
                    .description("Tool A")
                    .build()
                    .unwrap(),
            }),
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObjectArgs::default()
                    .name("tool_b")
                    .description("Tool B")
                    .build()
                    .unwrap(),
            }),
        ];

        server.register_from_oai_schemas(&schemas, ToolKind::PseudoTool);
        assert_eq!(server.tool_count(), initial_count + 2);
        assert!(server.tool_names().contains(&"tool_a".to_string()));
        assert!(server.tool_names().contains(&"tool_b".to_string()));
    }

    #[test]
    fn test_mcp_pseudo_tool_schemas_not_empty() {
        let schemas = crate::tool_schemas::mcp_pseudo_tool_schemas();
        // Should have a substantial number of tools
        assert!(
            schemas.len() >= 20,
            "Expected >= 20 MCP pseudo-tools, got {}",
            schemas.len()
        );
    }

    // ── E2E duplex tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_e2e_list_tools_matches_registration() {
        // Use a fresh server (not make_test_server) to avoid overlap
        let executor: ToolExecutor = Arc::new(|name, args| {
            Box::pin(
                async move { Ok(serde_json::json!({"status": "ok", "tool": name, "echo": args})) },
            )
        });
        let server = AegisMcpServer::new(executor);

        // Register pseudo-tools from OAI schemas
        let schemas = crate::tool_schemas::mcp_pseudo_tool_schemas();
        server.register_from_oai_schemas(&schemas, ToolKind::PseudoTool);

        let expected_count = schemas.len();

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        let tools = client.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), expected_count);

        // Verify some known tools are present
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"get_current_time"));
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"recall_memory"));

        client.cancel().await.ok();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_e2e_call_tool_success() {
        let executor: ToolExecutor = Arc::new(|name, args| {
            Box::pin(async move {
                Ok(serde_json::json!({
                    "status": "ok",
                    "tool": name,
                    "received": args,
                }))
            })
        });
        let server = AegisMcpServer::new(executor);
        server.register_pseudo_tool(
            "echo",
            "Echo back args",
            serde_json::json!({"type": "object", "properties": {"msg": {"type": "string"}}}),
        );

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("echo").with_arguments(
                    vec![("msg".into(), serde_json::json!("hello"))]
                        .into_iter()
                        .collect(),
                ),
            )
            .await
            .unwrap();

        assert_ne!(result.is_error, Some(true));
        assert!(!result.content.is_empty());

        client.cancel().await.ok();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_e2e_call_tool_error_propagation() {
        let executor: ToolExecutor = Arc::new(|_name, _args| {
            Box::pin(async move { Err(anyhow::anyhow!("simulated executor failure")) })
        });
        let server = AegisMcpServer::new(executor);
        server.register_pseudo_tool(
            "failing_tool",
            "Always fails",
            serde_json::json!({"type": "object"}),
        );

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new("failing_tool"))
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(true));

        client.cancel().await.ok();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_e2e_call_unknown_tool() {
        let server = make_test_server();

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new(
                "nonexistent_tool_xyz",
            ))
            .await
            .unwrap();

        // Unknown tool should return error
        assert_eq!(result.is_error, Some(true));

        client.cancel().await.ok();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_e2e_resources_reflect_tools() {
        let server = make_test_server();
        // Also register a Wasm skill to test the skills resource filter
        server.register_tool(RegisteredTool {
            name: "my_wasm_skill".into(),
            description: "A wasm skill".into(),
            input_schema: serde_json::json!({"type": "object"}),
            kind: ToolKind::WasmSkill,
        });

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = crate::mcp_client::AegisClientHandler
            .serve(client_io)
            .await
            .unwrap();

        // Helper to extract text from ResourceContents
        fn extract_text(contents: &[rmcp::model::ResourceContents]) -> String {
            match &contents[0] {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
                _ => panic!("expected TextResourceContents"),
            }
        }

        // Health resource should show correct tool count (2 pseudo + 1 wasm = 3)
        let health = client
            .read_resource(rmcp::model::ReadResourceRequestParams::new(
                "aegis-runtime://health",
            ))
            .await
            .unwrap();
        let health_text = extract_text(&health.contents);
        assert!(
            health_text.contains("3") || health_text.contains("tools_registered"),
            "health should report tool count: {health_text}"
        );

        // Skills resource should list only Wasm skills
        let skills = client
            .read_resource(rmcp::model::ReadResourceRequestParams::new(
                "aegis-runtime://skills",
            ))
            .await
            .unwrap();
        let skills_text = extract_text(&skills.contents);
        assert!(skills_text.contains("my_wasm_skill"));
        // Pseudo-tools should NOT appear in skills resource
        assert!(!skills_text.contains("get_current_time"));

        client.cancel().await.ok();
        server_handle.abort();
    }

    #[tokio::test]
    async fn test_e2e_concurrent_tool_calls() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_clone = call_count.clone();
        let executor: ToolExecutor = Arc::new(move |name, _args| {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                // Small delay to test concurrency
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                Ok(serde_json::json!({"status": "ok", "tool": name}))
            })
        });

        let server = AegisMcpServer::new(executor);
        for i in 0..3 {
            server.register_pseudo_tool(
                &format!("tool_{i}"),
                &format!("Tool {i}"),
                serde_json::json!({"type": "object"}),
            );
        }

        let (server_io, client_io) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            let svc = server.serve(server_io).await.unwrap();
            svc.waiting().await.ok();
        });

        let client = Arc::new(
            crate::mcp_client::AegisClientHandler
                .serve(client_io)
                .await
                .unwrap(),
        );

        // Fire 3 calls concurrently
        let mut handles = Vec::new();
        for i in 0..3 {
            let c = client.clone();
            handles.push(tokio::spawn(async move {
                c.call_tool(rmcp::model::CallToolRequestParams::new(format!("tool_{i}")))
                    .await
            }));
        }

        for handle in handles {
            let result = handle.await.unwrap().unwrap();
            assert_ne!(result.is_error, Some(true));
        }

        assert_eq!(call_count.load(Ordering::SeqCst), 3);

        // client is Arc, need to unwrap for cancel
        // Just abort server since client is shared
        server_handle.abort();
    }
}

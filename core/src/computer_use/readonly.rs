//! Bounded read-only Secure CU tools (Linux AT-SPI + loopback stubs).
//!
//! Pure helpers live here so unit tests do not need a session bus. Live
//! AT-SPI walks stay in [`super::linux`].

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Read-only CU logical tools (P0.6). Never `shell.exec` / `eval`.
pub const CU_READONLY_TOOLS: &[&str] = &[
    "system.session_probe",
    "system.frontmost_app",
    "system.accessible_tree",
    "system.focused_control",
    "system.screenshot_digest",
];

/// AT-SPI `AtspiStateType::ATSPI_STATE_FOCUSED` (0-based enum index).
pub const ATSPI_STATE_FOCUSED: u32 = 12;

/// Hard caps so a deep desktop tree cannot blow the receipt.
pub const DEFAULT_TREE_DEPTH: u32 = 2;
pub const MAX_TREE_DEPTH: u32 = 3;
pub const DEFAULT_TREE_NODES: usize = 32;
pub const MAX_TREE_NODES: usize = 64;

/// One accessible node after a bounded walk (testable without D-Bus).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessibleNode {
    pub bus_name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<AccessibleNode>,
}

impl AccessibleNode {
    pub fn key(&self) -> String {
        format!("{}:{}", self.bus_name, self.path)
    }
}

/// Clamp tree walk bounds from CU args.
pub fn clamp_tree_bounds(args: &Value) -> (u32, usize) {
    let depth = args
        .get("max_depth")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TREE_DEPTH as u64)
        .clamp(1, MAX_TREE_DEPTH as u64) as u32;
    let nodes = args
        .get("max_nodes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TREE_NODES as u64)
        .clamp(1, MAX_TREE_NODES as u64) as usize;
    (depth, nodes)
}

/// Whether the AT-SPI GetState bitset has FOCUSED set.
pub fn state_is_focused(lo: u32, hi: u32) -> bool {
    if ATSPI_STATE_FOCUSED < 32 {
        (lo & (1 << ATSPI_STATE_FOCUSED)) != 0
    } else {
        (hi & (1 << (ATSPI_STATE_FOCUSED - 32))) != 0
    }
}

/// Count nodes in a tree (root included).
pub fn count_nodes(node: &AccessibleNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

/// Depth-first search for the first focused control.
pub fn find_focused_control(node: &AccessibleNode) -> Option<&AccessibleNode> {
    if node.focused {
        return Some(node);
    }
    node.children.iter().find_map(find_focused_control)
}

/// Snapshot JSON for `system.accessible_tree` (pure).
pub fn snapshot_accessible_tree(root: &AccessibleNode, truncated: bool, max_depth: u32) -> Value {
    let digest = tree_digest(root);
    json!({
        "tree": root,
        "count": count_nodes(root),
        "truncated": truncated,
        "max_depth": max_depth,
        "tree_digest": digest,
        "note": "Bounded AT-SPI accessible tree (name/role/focus). No OS writes.",
    })
}

/// Snapshot JSON for `system.focused_control` (pure).
pub fn snapshot_focused_control(root: &AccessibleNode) -> Value {
    match find_focused_control(root) {
        Some(n) => json!({
            "found": true,
            "name": n.name,
            "role": n.role,
            "path": n.path,
            "bus_name": n.bus_name,
        }),
        None => json!({
            "found": false,
            "name": Value::Null,
            "role": Value::Null,
            "note": "No focused accessible in the bounded walk.",
        }),
    }
}

/// BLAKE3 hex of RFC 8785-ish canonical JSON (via serde_jcs).
pub fn tree_digest(root: &AccessibleNode) -> String {
    let bytes = serde_jcs::to_string(root)
        .unwrap_or_else(|_| "{}".to_string())
        .into_bytes();
    hex::encode(blake3::hash(&bytes).as_bytes())
}

/// BLAKE3 hex of raw bytes (screenshot stub / tests). Empty input is valid.
pub fn digest_bytes(bytes: &[u8]) -> String {
    hex::encode(blake3::hash(bytes).as_bytes())
}

/// Loopback stub payload for a read-only CU tool (no OS side effects).
pub fn loopback_readonly_result(tool: &str) -> Value {
    match tool {
        "system.accessible_tree" => {
            let root = AccessibleNode {
                bus_name: "loopback".into(),
                path: "/stub/root".into(),
                name: Some("Loopback desktop".into()),
                role: Some("desktop frame".into()),
                focused: false,
                children: vec![AccessibleNode {
                    bus_name: "loopback".into(),
                    path: "/stub/app".into(),
                    name: Some("Stub App".into()),
                    role: Some("application".into()),
                    focused: true,
                    children: vec![],
                }],
            };
            let mut snap = snapshot_accessible_tree(&root, false, DEFAULT_TREE_DEPTH);
            snap["adapter"] = json!("loopback");
            snap["status"] = json!("ok");
            snap["tool"] = json!(tool);
            snap
        }
        "system.focused_control" => json!({
            "status": "ok",
            "adapter": "loopback",
            "tool": tool,
            "found": true,
            "name": "Stub App",
            "role": "application",
            "path": "/stub/app",
            "bus_name": "loopback",
            "note": "stub — no OS side effects",
        }),
        "system.screenshot_digest" => json!({
            "status": "ok",
            "adapter": "loopback",
            "tool": tool,
            "digest": digest_bytes(b"loopback-screenshot-stub"),
            "path": Value::Null,
            "capture": "stub",
            "note": "stub hash — no pixels captured",
        }),
        _ => json!({
            "status": "ok",
            "adapter": "loopback",
            "tool": tool,
            "note": "stub — no OS side effects",
        }),
    }
}

/// True when `tool` is a known read-only CU logical tool.
pub fn is_cu_readonly_tool(tool: &str) -> bool {
    CU_READONLY_TOOLS.contains(&tool)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> AccessibleNode {
        AccessibleNode {
            bus_name: ":1.1".into(),
            path: "/root".into(),
            name: Some("Desktop".into()),
            role: Some("desktop frame".into()),
            focused: false,
            children: vec![
                AccessibleNode {
                    bus_name: ":1.2".into(),
                    path: "/app".into(),
                    name: Some("Terminal".into()),
                    role: Some("application".into()),
                    focused: false,
                    children: vec![AccessibleNode {
                        bus_name: ":1.2".into(),
                        path: "/app/entry".into(),
                        name: Some("command".into()),
                        role: Some("entry".into()),
                        focused: true,
                        children: vec![],
                    }],
                },
                AccessibleNode {
                    bus_name: ":1.3".into(),
                    path: "/other".into(),
                    name: None,
                    role: Some("filler".into()),
                    focused: false,
                    children: vec![],
                },
            ],
        }
    }

    #[test]
    fn clamp_tree_bounds_caps_depth_and_nodes() {
        assert_eq!(clamp_tree_bounds(&json!({})), (2, 32));
        assert_eq!(
            clamp_tree_bounds(&json!({"max_depth": 99, "max_nodes": 9999})),
            (3, 64)
        );
        assert_eq!(
            clamp_tree_bounds(&json!({"max_depth": 0, "max_nodes": 0})),
            (1, 1)
        );
    }

    #[test]
    fn focused_bit_matches_atspi_enum() {
        assert!(!state_is_focused(0, 0));
        assert!(state_is_focused(1 << ATSPI_STATE_FOCUSED, 0));
        assert!(!state_is_focused(1 << 11, 0));
    }

    #[test]
    fn tree_snapshot_is_bounded_and_hashed() {
        let root = sample_tree();
        let snap = snapshot_accessible_tree(&root, true, 2);
        assert_eq!(snap["count"], 4);
        assert_eq!(snap["truncated"], true);
        assert_eq!(snap["tree_digest"].as_str().unwrap().len(), 64);
        let focused = snapshot_focused_control(&root);
        assert_eq!(focused["found"], true);
        assert_eq!(focused["name"], "command");
        assert_eq!(focused["role"], "entry");
    }

    #[test]
    fn focused_control_missing_is_explicit() {
        let root = AccessibleNode {
            bus_name: "b".into(),
            path: "/p".into(),
            name: Some("x".into()),
            role: Some("panel".into()),
            focused: false,
            children: vec![],
        };
        let snap = snapshot_focused_control(&root);
        assert_eq!(snap["found"], false);
    }

    #[test]
    fn digest_bytes_is_stable() {
        assert_eq!(digest_bytes(b"abc"), digest_bytes(b"abc"));
        assert_ne!(digest_bytes(b"abc"), digest_bytes(b"abd"));
    }

    #[test]
    fn known_readonly_tools_listed() {
        assert!(is_cu_readonly_tool("system.accessible_tree"));
        assert!(is_cu_readonly_tool("system.focused_control"));
        assert!(is_cu_readonly_tool("system.screenshot_digest"));
        assert!(!is_cu_readonly_tool("shell.exec"));
    }
}

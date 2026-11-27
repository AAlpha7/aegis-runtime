//! Linux Device Agent — AT-SPI session adapter for Ubuntu desktop hosts.
//!
//! Feature-detects display + session bus + `org.a11y.Bus` (AT-SPI). Headless
//! cloud / missing AT-SPI returns [`DeviceRejectReason::AdapterUnavailable`]
//! (no panic). Protocol safety (verify / bind / lease / replay) is identical
//! to [`super::device::LoopbackDeviceAgent`] via [`ProtocolGate`].

use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::computer_use::device::{DeviceAgent, DeviceReceipt, DeviceRejectReason, ProtocolGate};
use crate::computer_use::intent::CuIntentEnvelope;
#[cfg(target_os = "linux")]
use crate::computer_use::readonly::state_is_focused;
use crate::computer_use::readonly::{
    AccessibleNode, clamp_tree_bounds, snapshot_accessible_tree, snapshot_focused_control,
};

/// Result of probing the local Linux accessibility session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinuxSessionProbe {
    pub available: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wayland_display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dbus_session_bus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atspi_bus_address: Option<String>,
    pub platform: String,
}

/// Pluggable UI backend so unit tests can inject success / failure without
/// a real AT-SPI session.
pub trait LinuxUiBackend: Send + Sync {
    fn probe(&self) -> LinuxSessionProbe;
    /// Best-effort desktop application snapshot via AT-SPI Registry.
    fn frontmost_app(&self) -> Result<Value, String>;
    /// Bounded accessible tree (name/role/focus). Headless backends return Err.
    fn accessible_tree(&self, max_depth: u32, max_nodes: usize) -> Result<Value, String>;
    /// Focused control name/role if AT-SPI exposes STATE_FOCUSED.
    fn focused_control(&self) -> Result<Value, String>;
    /// Optional screenshot digest. Capture is best-effort; Err → adapter_unavailable.
    fn screenshot_digest(&self) -> Result<Value, String>;
}

/// Live backend: probes env + (on Linux) `org.a11y.Bus` over the session bus.
#[derive(Debug, Default, Clone)]
pub struct LiveAtspiBackend;

impl LinuxUiBackend for LiveAtspiBackend {
    fn probe(&self) -> LinuxSessionProbe {
        probe_linux_session()
    }

    fn frontmost_app(&self) -> Result<Value, String> {
        let probe = probe_linux_session();
        if !probe.available {
            return Err(probe.reason);
        }
        query_registry_applications(&probe)
    }

    fn accessible_tree(&self, max_depth: u32, max_nodes: usize) -> Result<Value, String> {
        let probe = probe_linux_session();
        if !probe.available {
            return Err(probe.reason);
        }
        query_accessible_tree(&probe, max_depth, max_nodes)
    }

    fn focused_control(&self) -> Result<Value, String> {
        let probe = probe_linux_session();
        if !probe.available {
            return Err(probe.reason);
        }
        query_focused_control(&probe)
    }

    fn screenshot_digest(&self) -> Result<Value, String> {
        let probe = probe_linux_session();
        if !probe.available {
            return Err(probe.reason);
        }
        // Pixel capture needs grim/import and is deferred. When AT-SPI is up
        // we still emit a tree_digest so audit has a stable hash (no pixels).
        let tree = query_accessible_tree(&probe, 1, 16)?;
        let digest = tree
            .get("tree_digest")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok(json!({
            "digest": digest,
            "path": Value::Null,
            "capture": "tree_proxy",
            "note": "Raw screenshot pixels deferred (no grim/import). digest is blake3 of a depth-1 accessible tree.",
        }))
    }
}

/// Test / forced-unavailable backend.
#[derive(Debug, Clone)]
pub struct StaticLinuxBackend {
    pub probe: LinuxSessionProbe,
    pub frontmost: Result<Value, String>,
    pub tree: Result<Value, String>,
    pub focused: Result<Value, String>,
    pub screenshot: Result<Value, String>,
}

impl LinuxUiBackend for StaticLinuxBackend {
    fn probe(&self) -> LinuxSessionProbe {
        self.probe.clone()
    }

    fn frontmost_app(&self) -> Result<Value, String> {
        self.frontmost.clone()
    }

    fn accessible_tree(&self, _max_depth: u32, _max_nodes: usize) -> Result<Value, String> {
        self.tree.clone()
    }

    fn focused_control(&self) -> Result<Value, String> {
        self.focused.clone()
    }

    fn screenshot_digest(&self) -> Result<Value, String> {
        self.screenshot.clone()
    }
}

/// Linux Device Agent (AT-SPI).
pub struct LinuxAtspiDeviceAgent {
    gate: ProtocolGate,
    backend: Arc<dyn LinuxUiBackend>,
}

impl LinuxAtspiDeviceAgent {
    pub fn new(device_id: impl Into<String>) -> Self {
        Self {
            gate: ProtocolGate::new(device_id),
            backend: Arc::new(LiveAtspiBackend),
        }
    }

    pub fn with_backend(device_id: impl Into<String>, backend: Arc<dyn LinuxUiBackend>) -> Self {
        Self {
            gate: ProtocolGate::new(device_id),
            backend,
        }
    }

    pub fn accept_ask(mut self, accept: bool) -> Self {
        self.gate = self.gate.accept_ask(accept);
        self
    }

    pub fn set_now_for_test(&self, now: u64) {
        self.gate.set_now_for_test(now);
    }

    pub fn probe(&self) -> LinuxSessionProbe {
        self.backend.probe()
    }

    fn execute_readonly_atspi(
        &self,
        envelope: &CuIntentEnvelope,
        tool: &str,
        now: u64,
    ) -> Result<DeviceReceipt> {
        let probe = self.backend.probe();
        if !probe.available {
            return Ok(self.gate.reject_with_result(
                envelope,
                DeviceRejectReason::AdapterUnavailable,
                now,
                json!({
                    "status": "rejected",
                    "adapter": "linux_atspi",
                    "tool": tool,
                    "probe": probe,
                    "hint": "need a graphical session with AT-SPI (org.a11y.Bus); headless cloud returns adapter_unavailable",
                }),
            ));
        }

        let outcome = match tool {
            "system.frontmost_app" => self.backend.frontmost_app().map(|app| json!({"app": app})),
            "system.accessible_tree" => {
                let (depth, nodes) = clamp_tree_bounds(&envelope.body.args);
                self.backend
                    .accessible_tree(depth, nodes)
                    .map(|tree| json!({"tree": tree}))
            }
            "system.focused_control" => self
                .backend
                .focused_control()
                .map(|focused| json!({"focused": focused})),
            "system.screenshot_digest" => self
                .backend
                .screenshot_digest()
                .map(|shot| json!({"screenshot": shot})),
            _ => Err("unsupported_readonly_tool".into()),
        };

        match outcome {
            Ok(payload) => {
                let mut result = json!({
                    "status": "ok",
                    "adapter": "linux_atspi",
                    "tool": tool,
                    "probe": probe,
                });
                if let Some(obj) = payload.as_object() {
                    for (k, v) in obj {
                        result[k] = v.clone();
                    }
                }
                Ok(self.gate.accept(envelope, now, result))
            }
            Err(err) => Ok(self.gate.reject_with_result(
                envelope,
                DeviceRejectReason::AdapterUnavailable,
                now,
                json!({
                    "status": "rejected",
                    "adapter": "linux_atspi",
                    "tool": tool,
                    "error": err,
                    "probe": probe,
                }),
            )),
        }
    }
}

impl DeviceAgent for LinuxAtspiDeviceAgent {
    fn device_id(&self) -> &str {
        self.gate.device_id()
    }

    fn execute(&self, envelope: &CuIntentEnvelope) -> Result<DeviceReceipt> {
        let now = match self.gate.validate(envelope) {
            Ok(n) => n,
            Err(reason) => return Ok(self.gate.reject(envelope, reason, self.gate.now())),
        };

        let tool = envelope.body.tool.as_str();
        if tool.contains("shell") || tool.contains("eval") {
            return Ok(self
                .gate
                .reject(envelope, DeviceRejectReason::UnsupportedTool, now));
        }

        match tool {
            "system.session_probe" => {
                let probe = self.backend.probe();
                Ok(self.gate.accept(
                    envelope,
                    now,
                    json!({
                        "status": "ok",
                        "adapter": "linux_atspi",
                        "tool": tool,
                        "probe": probe,
                    }),
                ))
            }
            "system.frontmost_app"
            | "system.accessible_tree"
            | "system.focused_control"
            | "system.screenshot_digest" => self.execute_readonly_atspi(envelope, tool, now),
            _ => Ok(self
                .gate
                .reject(envelope, DeviceRejectReason::UnsupportedTool, now)),
        }
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Probe display + (Linux) AT-SPI bus. Never panics.
pub fn probe_linux_session() -> LinuxSessionProbe {
    let display = env_nonempty("DISPLAY");
    let wayland_display = env_nonempty("WAYLAND_DISPLAY");
    let dbus_session_bus = env_nonempty("DBUS_SESSION_BUS_ADDRESS");
    let platform = std::env::consts::OS.to_string();

    #[cfg(not(target_os = "linux"))]
    {
        return LinuxSessionProbe {
            available: false,
            reason: format!("not_linux (platform={platform})"),
            display,
            wayland_display,
            dbus_session_bus,
            atspi_bus_address: None,
            platform,
        };
    }

    #[cfg(target_os = "linux")]
    {
        if display.is_none() && wayland_display.is_none() {
            return LinuxSessionProbe {
                available: false,
                reason: "no_display (set DISPLAY or WAYLAND_DISPLAY in a desktop session)".into(),
                display,
                wayland_display,
                dbus_session_bus,
                atspi_bus_address: None,
                platform,
            };
        }

        match try_atspi_bus_address() {
            Ok(addr) => LinuxSessionProbe {
                available: true,
                reason: "ok".into(),
                display,
                wayland_display,
                dbus_session_bus,
                atspi_bus_address: Some(addr),
                platform,
            },
            Err(reason) => LinuxSessionProbe {
                available: false,
                reason,
                display,
                wayland_display,
                dbus_session_bus,
                atspi_bus_address: None,
                platform,
            },
        }
    }
}

#[cfg(target_os = "linux")]
fn try_atspi_bus_address() -> Result<String, String> {
    let conn = zbus::blocking::Connection::session()
        .map_err(|e| format!("session_bus_unavailable: {e}"))?;
    let proxy = zbus::blocking::Proxy::new(&conn, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
        .map_err(|e| format!("a11y_bus_proxy: {e}"))?;
    let address: String = proxy
        .call("GetAddress", &())
        .map_err(|e| format!("a11y_GetAddress_failed: {e}"))?;
    if address.trim().is_empty() {
        return Err("a11y_GetAddress_empty".into());
    }
    // Confirm the AT-SPI bus itself is reachable.
    let _a11y = zbus::blocking::connection::Builder::address(address.as_str())
        .map_err(|e| format!("a11y_builder: {e}"))?
        .build()
        .map_err(|e| format!("a11y_bus_connect: {e}"))?;
    Ok(address)
}

/// Build Accessible.GetChildren fallback snapshot (pure; no D-Bus).
///
/// `children` is `(bus_name, object_path, optional Name)` — name enrichment is
/// best-effort and may be `None` when the Accessible.Name property is missing.
fn snapshot_from_accessible_children(
    children: impl IntoIterator<Item = (String, String, Option<String>)>,
) -> Value {
    let mut application_paths = Vec::new();
    let mut applications = Vec::new();
    for (bus_name, path_str, maybe_name) in children.into_iter().take(64) {
        let key = format!("{bus_name}:{path_str}");
        application_paths.push(key);
        let name = match maybe_name {
            Some(n) if !n.trim().is_empty() => Value::String(n),
            _ => Value::Null,
        };
        applications.push(json!({
            "bus_name": bus_name,
            "path": path_str,
            "name": name,
        }));
    }

    let active = applications
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()).is_some())
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "active": active,
        "application_paths": application_paths,
        "applications": applications,
        "count": application_paths.len(),
        "note": "AT-SPI Accessible.GetChildren fallback (at-spi2 without Registry.GetApplications).",
    })
}

fn query_registry_applications(probe: &LinuxSessionProbe) -> Result<Value, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = probe;
        return Err("not_linux".into());
    }

    #[cfg(target_os = "linux")]
    {
        let Some(addr) = probe.atspi_bus_address.as_deref() else {
            return Err("no_atspi_address".into());
        };
        let conn = zbus::blocking::connection::Builder::address(addr)
            .map_err(|e| format!("a11y_builder: {e}"))?
            .build()
            .map_err(|e| format!("a11y_bus_connect: {e}"))?;

        // Prefer legacy Registry.GetApplications (ao). at-spi2-core ≥2.52 dropped it;
        // fall back to Accessible.GetChildren on the desktop root (a(so)).
        let registry = zbus::blocking::Proxy::new(
            &conn,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/registry",
            "org.a11y.atspi.Registry",
        )
        .map_err(|e| format!("registry_proxy: {e}"))?;

        let legacy: Result<Vec<zbus::zvariant::OwnedObjectPath>, _> =
            registry.call("GetApplications", &());
        if let Ok(apps) = legacy {
            let paths: Vec<String> = apps.iter().take(64).map(|p| p.to_string()).collect();
            return Ok(json!({
                "active": Value::Null,
                "application_paths": paths,
                "count": apps.len(),
                "note": "AT-SPI Registry.GetApplications snapshot (paths). Name/active enrichment is best-effort later.",
            }));
        }

        let root = zbus::blocking::Proxy::new(
            &conn,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/accessible/root",
            "org.a11y.atspi.Accessible",
        )
        .map_err(|e| format!("root_accessible_proxy: {e}"))?;

        let children: Vec<(String, zbus::zvariant::OwnedObjectPath)> = root
            .call("GetChildren", &())
            .map_err(|e| format!("root_GetChildren_failed: {e}"))?;

        let mut enriched = Vec::new();
        for (bus_name, path) in children.into_iter().take(64) {
            let path_str = path.to_string();
            let mut maybe_name = None;
            if let Ok(app_proxy) = zbus::blocking::Proxy::new(
                &conn,
                bus_name.as_str(),
                path_str.as_str(),
                "org.a11y.atspi.Accessible",
            ) && let Ok(n) = app_proxy.get_property::<String>("Name")
                && !n.trim().is_empty()
            {
                maybe_name = Some(n);
            }
            enriched.push((bus_name, path_str, maybe_name));
        }

        Ok(snapshot_from_accessible_children(enriched))
    }
}

fn query_accessible_tree(
    probe: &LinuxSessionProbe,
    max_depth: u32,
    max_nodes: usize,
) -> Result<Value, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (probe, max_depth, max_nodes);
        return Err("not_linux".into());
    }

    #[cfg(target_os = "linux")]
    {
        let Some(addr) = probe.atspi_bus_address.as_deref() else {
            return Err("no_atspi_address".into());
        };
        let conn = zbus::blocking::connection::Builder::address(addr)
            .map_err(|e| format!("a11y_builder: {e}"))?
            .build()
            .map_err(|e| format!("a11y_bus_connect: {e}"))?;

        let mut remaining = max_nodes;
        let mut truncated = false;
        let root = walk_accessible(
            &conn,
            "org.a11y.atspi.Registry",
            "/org/a11y/atspi/accessible/root",
            0,
            max_depth,
            &mut remaining,
            &mut truncated,
        )?;
        Ok(snapshot_accessible_tree(&root, truncated, max_depth))
    }
}

fn query_focused_control(probe: &LinuxSessionProbe) -> Result<Value, String> {
    let snap = query_accessible_tree(probe, 2, 48)?;
    let tree = snap.get("tree").cloned().unwrap_or(Value::Null);
    let root: AccessibleNode = serde_json::from_value(tree).unwrap_or(AccessibleNode {
        bus_name: "org.a11y.atspi.Registry".into(),
        path: "/org/a11y/atspi/accessible/root".into(),
        name: None,
        role: None,
        focused: false,
        children: vec![],
    });
    Ok(snapshot_focused_control(&root))
}

#[cfg(target_os = "linux")]
fn walk_accessible(
    conn: &zbus::blocking::Connection,
    bus_name: &str,
    path: &str,
    depth: u32,
    max_depth: u32,
    remaining: &mut usize,
    truncated: &mut bool,
) -> Result<AccessibleNode, String> {
    if *remaining == 0 {
        *truncated = true;
        return Ok(AccessibleNode {
            bus_name: bus_name.into(),
            path: path.into(),
            name: None,
            role: None,
            focused: false,
            children: vec![],
        });
    }
    *remaining = remaining.saturating_sub(1);

    let proxy = zbus::blocking::Proxy::new(conn, bus_name, path, "org.a11y.atspi.Accessible")
        .map_err(|e| format!("accessible_proxy: {e}"))?;

    let name = proxy
        .get_property::<String>("Name")
        .ok()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let role: Option<String> = proxy
        .call("GetRoleName", &())
        .ok()
        .map(|n: String| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let focused = match proxy.call("GetState", &()) {
        Ok((lo, hi)) => state_is_focused(lo, hi),
        Err(_) => false,
    };

    let mut children = Vec::new();
    if depth < max_depth {
        let kids: Result<Vec<(String, zbus::zvariant::OwnedObjectPath)>, _> =
            proxy.call("GetChildren", &());
        if let Ok(kids) = kids {
            for (child_bus, child_path) in kids.into_iter().take(16) {
                if *remaining == 0 {
                    *truncated = true;
                    break;
                }
                match walk_accessible(
                    conn,
                    &child_bus,
                    child_path.as_str(),
                    depth + 1,
                    max_depth,
                    remaining,
                    truncated,
                ) {
                    Ok(child) => children.push(child),
                    Err(_) => continue,
                }
            }
        }
    } else {
        let kids: Result<Vec<(String, zbus::zvariant::OwnedObjectPath)>, _> =
            proxy.call("GetChildren", &());
        if kids.map(|k| !k.is_empty()).unwrap_or(false) {
            *truncated = true;
        }
    }

    Ok(AccessibleNode {
        bus_name: bus_name.into(),
        path: path.into(),
        name,
        role,
        focused,
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::intent::CuIntentBody;
    use crate::computer_use::policy::PolicyVerdict;
    use crate::computer_use::signer::CuSigner;
    use crate::identity::AgentIdentity;
    use serde_json::json;

    fn sign_tool(device_id: &str, tool: &str) -> CuIntentEnvelope {
        let identity = AgentIdentity::generate();
        let now = 1_700_000_000_u64;
        let body = CuIntentBody::new(
            "intent-linux",
            identity.did.clone(),
            device_id,
            tool,
            json!({}),
            now,
            300,
            PolicyVerdict::Allow,
        );
        CuSigner::new(&identity).sign(body).unwrap()
    }

    fn unavailable_backend() -> Arc<dyn LinuxUiBackend> {
        Arc::new(StaticLinuxBackend {
            probe: LinuxSessionProbe {
                available: false,
                reason: "no_display (test)".into(),
                display: None,
                wayland_display: None,
                dbus_session_bus: None,
                atspi_bus_address: None,
                platform: "linux".into(),
            },
            frontmost: Err("no_display (test)".into()),
            tree: Err("no_display (test)".into()),
            focused: Err("no_display (test)".into()),
            screenshot: Err("no_display (test)".into()),
        })
    }

    fn available_backend() -> Arc<dyn LinuxUiBackend> {
        Arc::new(StaticLinuxBackend {
            probe: LinuxSessionProbe {
                available: true,
                reason: "ok".into(),
                display: Some(":0".into()),
                wayland_display: None,
                dbus_session_bus: Some("unix:path=/tmp/bus".into()),
                atspi_bus_address: Some("unix:abstract=/tmp/at-spi".into()),
                platform: "linux".into(),
            },
            frontmost: Ok(json!({
                "active": Value::Null,
                "application_paths": ["/org/a11y/atspi/accessible/1"],
                "count": 1,
            })),
            tree: Ok(json!({
                "count": 2,
                "truncated": false,
                "tree_digest": "abc",
                "tree": {"name": "Desktop", "role": "desktop frame"},
            })),
            focused: Ok(json!({
                "found": true,
                "name": "command",
                "role": "entry",
            })),
            screenshot: Ok(json!({
                "digest": "deadbeef",
                "path": Value::Null,
                "capture": "tree_proxy",
            })),
        })
    }

    #[test]
    fn session_probe_succeeds_when_atspi_missing() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", unavailable_backend());
        agent.set_now_for_test(1_700_000_000);
        let env = sign_tool("dev-linux", "system.session_probe");
        let receipt = agent.execute(&env).unwrap();
        assert!(receipt.accepted);
        assert_eq!(receipt.result["probe"]["available"], false);
    }

    #[test]
    fn frontmost_app_rejects_cleanly_without_atspi() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", unavailable_backend());
        agent.set_now_for_test(1_700_000_000);
        let env = sign_tool("dev-linux", "system.frontmost_app");
        let receipt = agent.execute(&env).unwrap();
        assert!(!receipt.accepted);
        assert_eq!(
            receipt.reject_reason,
            Some(DeviceRejectReason::AdapterUnavailable)
        );
    }

    #[test]
    fn frontmost_app_ok_with_mock_backend() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", available_backend());
        agent.set_now_for_test(1_700_000_000);
        let env = sign_tool("dev-linux", "system.frontmost_app");
        let receipt = agent.execute(&env).unwrap();
        assert!(receipt.accepted);
        assert_eq!(receipt.result["app"]["count"], 1);
    }

    #[test]
    fn rejects_replay_like_loopback() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", available_backend());
        agent.set_now_for_test(1_700_000_000);
        let env = sign_tool("dev-linux", "system.session_probe");
        assert!(agent.execute(&env).unwrap().accepted);
        let second = agent.execute(&env).unwrap();
        assert_eq!(second.reject_reason, Some(DeviceRejectReason::Replay));
    }

    #[test]
    fn live_probe_does_not_panic_headless() {
        let probe = probe_linux_session();
        assert!(!probe.reason.is_empty());
        assert_eq!(probe.platform, std::env::consts::OS);
    }

    #[test]
    fn new_readonly_tools_ok_with_mock_backend() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", available_backend());
        agent.set_now_for_test(1_700_000_000);
        for tool in [
            "system.accessible_tree",
            "system.focused_control",
            "system.screenshot_digest",
        ] {
            let env = sign_tool("dev-linux", tool);
            let receipt = agent.execute(&env).unwrap();
            assert!(receipt.accepted, "{tool} should accept on mock backend");
        }
    }

    #[test]
    fn new_readonly_tools_reject_cleanly_without_atspi() {
        let agent = LinuxAtspiDeviceAgent::with_backend("dev-linux", unavailable_backend());
        agent.set_now_for_test(1_700_000_000);
        let env = sign_tool("dev-linux", "system.accessible_tree");
        let receipt = agent.execute(&env).unwrap();
        assert!(!receipt.accepted);
        assert_eq!(
            receipt.reject_reason,
            Some(DeviceRejectReason::AdapterUnavailable)
        );
    }

    #[test]
    fn get_children_fallback_snapshot_without_bus() {
        // Pure helper: no live AT-SPI / D-Bus required.
        let snap = snapshot_from_accessible_children(vec![
            (
                ":1.42".into(),
                "/org/a11y/atspi/accessible/1".into(),
                Some("Terminal".into()),
            ),
            (":1.43".into(), "/org/a11y/atspi/accessible/2".into(), None),
        ]);
        assert_eq!(snap["count"], 2);
        assert_eq!(
            snap["note"],
            "AT-SPI Accessible.GetChildren fallback (at-spi2 without Registry.GetApplications)."
        );
        assert_eq!(
            snap["application_paths"][0],
            ":1.42:/org/a11y/atspi/accessible/1"
        );
        assert_eq!(snap["active"]["name"], "Terminal");
        assert_eq!(snap["applications"][1]["name"], Value::Null);
    }
}

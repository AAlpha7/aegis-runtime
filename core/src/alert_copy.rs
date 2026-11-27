//! Owner-facing heartbeat alert copy.
//!
//! Skill payloads (English metric dumps, `[STILL ONGOING]` / `[RESOLVED]` /
//! `Error:` tokens) stay as chronos wrote them. This module translates at
//! render time so Telegram and the Web console speak the owner's language.

use std::sync::LazyLock;

use regex::Regex;

use crate::i18n::{t, tfp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertState {
    New,
    Ongoing,
    Resolved,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedAlert {
    pub title: String,
    pub body: String,
    pub state: AlertState,
}

pub fn alert_state(payload: &str) -> AlertState {
    if payload.contains("[RESOLVED]") {
        AlertState::Resolved
    } else if payload.contains("[STILL ONGOING]") {
        AlertState::Ongoing
    } else if payload.starts_with("Error:") {
        AlertState::Error
    } else {
        AlertState::New
    }
}

pub fn localize_heartbeat_alert(skill_name: &str, payload: &str, locale: &str) -> LocalizedAlert {
    let state = alert_state(payload);
    if is_server_monitor(skill_name) {
        localize_server_monitor(payload, state, locale)
    } else {
        localize_unknown_skill(skill_name, payload, state, locale)
    }
}

/// True when a server_monitor payload reports the disk over its threshold in
/// an active (new/ongoing) alert — the trigger for the "largest directories"
/// enrichment appended by the alert dispatcher (owner 2026-09-12).
pub fn payload_has_disk_pressure(skill_name: &str, payload: &str) -> bool {
    if !is_server_monitor(skill_name)
        || !matches!(alert_state(payload), AlertState::New | AlertState::Ongoing)
    {
        return false;
    }
    // In the "Action needed" section the skill has already decided the disk
    // needs attention, so allow the half-point the rounded display hides
    // (the 2026-09-12 10:40 alert read "70%" against a 70% line and went out
    // without the directory block). Metrics listed elsewhere count only when
    // over the line.
    let parsed = parse_server_payload(payload);
    parsed.action.iter().any(
        |metric| matches!(metric, Metric::Disk { percent, threshold, .. } if percent + 0.5 >= *threshold),
    ) || parsed.other.iter().any(
            |metric| matches!(metric, Metric::Disk { percent, threshold, .. } if percent >= threshold),
        )
}

fn is_server_monitor(skill_name: &str) -> bool {
    normalized_skill_name(skill_name) == "server_monitor"
}

fn normalized_skill_name(skill_name: &str) -> &str {
    skill_name.trim().trim_start_matches("skill_")
}

fn localize_unknown_skill(
    skill_name: &str,
    payload: &str,
    state: AlertState,
    locale: &str,
) -> LocalizedAlert {
    let display = skill_display_name(skill_name, locale);
    let title = match state {
        AlertState::Resolved => tfp(
            "alert.heartbeat.unknown.title_resolved",
            locale,
            &[&display],
        ),
        AlertState::Error => tfp("alert.heartbeat.unknown.title_error", locale, &[&display]),
        AlertState::New => tfp("alert.heartbeat.unknown.title", locale, &[&display]),
        AlertState::Ongoing => format!(
            "{}{}",
            tfp("alert.heartbeat.unknown.title", locale, &[&display]),
            t("alert.heartbeat.ongoing_suffix", locale)
        ),
    };
    let body = if state == AlertState::Error {
        error_body(payload)
    } else {
        strip_state_markers(payload)
    };
    LocalizedAlert { title, body, state }
}

fn skill_display_name(skill_name: &str, locale: &str) -> String {
    let normalized = normalized_skill_name(skill_name);
    let key = format!("console.source.{normalized}");
    let translated = t(&key, locale);
    if translated == "[MISSING]" {
        crate::evidence::humanize_machine_label(normalized)
    } else {
        translated.to_string()
    }
}

fn localize_server_monitor(payload: &str, state: AlertState, locale: &str) -> LocalizedAlert {
    if state == AlertState::Error {
        return LocalizedAlert {
            title: t("alert.heartbeat.server.title_error", locale).to_string(),
            body: error_body(payload),
            state,
        };
    }

    let parsed = parse_server_payload(payload);
    let title = server_title(state, &parsed.action, locale);
    let body = server_body(state, &parsed, locale);
    LocalizedAlert { title, body, state }
}

fn server_title(state: AlertState, action: &[Metric], locale: &str) -> String {
    match state {
        AlertState::Resolved => t("alert.heartbeat.server.title_resolved", locale).to_string(),
        AlertState::Error => t("alert.heartbeat.server.title_error", locale).to_string(),
        AlertState::New | AlertState::Ongoing => {
            let issues = issue_labels(action, locale);
            let suffix = if state == AlertState::Ongoing {
                t("alert.heartbeat.ongoing_suffix", locale)
            } else {
                ""
            };
            if issues.is_empty() {
                format!("{}{suffix}", t("alert.heartbeat.server.generic", locale))
            } else {
                format!(
                    "{}{}{suffix}",
                    t("alert.heartbeat.server.prefix", locale),
                    issues.join(t("alert.heartbeat.join", locale))
                )
            }
        }
    }
}

fn issue_labels(metrics: &[Metric], locale: &str) -> Vec<String> {
    let mut labels = Vec::new();
    if metrics.iter().any(|m| matches!(m, Metric::Disk { .. })) {
        labels.push(t("alert.heartbeat.issue.disk", locale).to_string());
    }
    if metrics.iter().any(|m| matches!(m, Metric::Memory { .. })) {
        labels.push(t("alert.heartbeat.issue.memory", locale).to_string());
    }
    if metrics.iter().any(|m| matches!(m, Metric::Cpu { .. })) {
        labels.push(t("alert.heartbeat.issue.cpu", locale).to_string());
    }
    labels
}

fn server_body(state: AlertState, parsed: &ParsedServer, locale: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if state == AlertState::Resolved {
        let metrics: Vec<&Metric> = if !parsed.all_ok.is_empty() {
            parsed.all_ok.iter().collect()
        } else {
            parsed.action.iter().chain(parsed.other.iter()).collect()
        };
        if !metrics.is_empty() {
            let joined = metrics
                .iter()
                .map(|m| metric_ok_line(m, locale))
                .collect::<Vec<_>>()
                .join(t("alert.heartbeat.clause_join", locale));
            parts.push(tfp("alert.heartbeat.resolved.sentence", locale, &[&joined]));
        }
    } else {
        for metric in ordered_action(&parsed.action) {
            parts.push(metric_action_line(metric, locale));
        }
        for metric in ordered_action(&parsed.action) {
            parts.push(suggestion_line(metric, locale));
        }
        if !parsed.other.is_empty() {
            let joined = parsed
                .other
                .iter()
                .map(|m| metric_ok_line(m, locale))
                .collect::<Vec<_>>()
                .join(t("alert.heartbeat.clause_join", locale));
            parts.push(tfp("alert.heartbeat.other.sentence", locale, &[&joined]));
        }
    }
    if !parsed.unparsed.is_empty() {
        parts.push(parsed.unparsed.join("\n"));
    }
    parts.join("\n")
}

fn ordered_action(action: &[Metric]) -> Vec<&Metric> {
    let mut out = Vec::new();
    out.extend(action.iter().filter(|m| matches!(m, Metric::Disk { .. })));
    out.extend(action.iter().filter(|m| matches!(m, Metric::Memory { .. })));
    out.extend(action.iter().filter(|m| matches!(m, Metric::Cpu { .. })));
    out
}

fn metric_action_line(metric: &Metric, locale: &str) -> String {
    match metric {
        Metric::Disk {
            percent,
            threshold,
            available,
            total,
        } => {
            let percent = format_percent(*percent);
            let threshold = format_percent(*threshold);
            tfp(
                "alert.heartbeat.disk.action",
                locale,
                &[&percent, available, total, &threshold],
            )
        }
        Metric::Memory {
            percent,
            threshold,
            available_mb,
            total_mb,
        } => {
            let percent = format_percent(*percent);
            let available = format_mb(*available_mb);
            let total = format_mb(*total_mb);
            let threshold = format_percent(*threshold);
            tfp(
                "alert.heartbeat.memory.action",
                locale,
                &[&percent, &available, &total, &threshold],
            )
        }
        Metric::Cpu {
            load1,
            load5,
            load15,
            limit,
            cpus,
        } => {
            let load1 = format_load(*load1);
            let load5 = format_load(*load5);
            let load15 = format_load(*load15);
            let cpus = cpus.to_string();
            let limit = format_limit(*limit);
            tfp(
                "alert.heartbeat.cpu.action",
                locale,
                &[&load1, &load5, &load15, &cpus, &limit],
            )
        }
    }
}

fn suggestion_line(metric: &Metric, locale: &str) -> String {
    let key = match metric {
        Metric::Disk { .. } => "alert.heartbeat.suggest.disk",
        Metric::Memory { .. } => "alert.heartbeat.suggest.memory",
        Metric::Cpu { .. } => "alert.heartbeat.suggest.cpu",
    };
    t(key, locale).to_string()
}

fn metric_ok_line(metric: &Metric, locale: &str) -> String {
    match metric {
        Metric::Disk {
            percent, available, ..
        } => {
            let percent = format_percent(*percent);
            tfp("alert.heartbeat.disk.ok", locale, &[&percent, available])
        }
        Metric::Memory {
            percent,
            available_mb,
            ..
        } => {
            let percent = format_percent(*percent);
            let available = format_mb(*available_mb);
            tfp("alert.heartbeat.memory.ok", locale, &[&percent, &available])
        }
        Metric::Cpu {
            load1,
            load5,
            load15,
            limit,
            cpus,
        } => {
            let load1 = format_load(*load1);
            let load5 = format_load(*load5);
            let load15 = format_load(*load15);
            let cpus = cpus.to_string();
            let limit = format_limit(*limit);
            tfp(
                "alert.heartbeat.cpu.ok",
                locale,
                &[&load1, &load5, &load15, &cpus, &limit],
            )
        }
    }
}

fn format_percent(value: f64) -> String {
    format!("{}", value.round() as i64)
}

fn format_load(value: f64) -> String {
    format!("{:.2}", value)
}

fn format_limit(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 {
        format!("{}", value.round() as i64)
    } else {
        format!("{:.1}", value)
    }
}

/// Owner-facing GB uses 1000 MB so a screenshot value like 19728 MB reads as 19.7 GB.
fn format_mb(mb: f64) -> String {
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else {
        format!("{} MB", mb.round() as i64)
    }
}

fn error_body(payload: &str) -> String {
    payload
        .strip_prefix("Error:")
        .unwrap_or(payload)
        .trim()
        .to_string()
}

fn strip_state_markers(payload: &str) -> String {
    payload
        .replace("[STILL ONGOING] ", "")
        .replace("[STILL ONGOING]", "")
        .replace("[RESOLVED] ", "")
        .replace("[RESOLVED]", "")
        .trim()
        .to_string()
}

#[derive(Debug, Clone)]
enum Metric {
    Disk {
        percent: f64,
        threshold: f64,
        available: String,
        total: String,
    },
    Memory {
        percent: f64,
        threshold: f64,
        available_mb: f64,
        total_mb: f64,
    },
    Cpu {
        load1: f64,
        load5: f64,
        load15: f64,
        limit: f64,
        cpus: u32,
    },
}

#[derive(Debug, Default)]
struct ParsedServer {
    action: Vec<Metric>,
    other: Vec<Metric>,
    all_ok: Vec<Metric>,
    unparsed: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Action,
    Steps,
    Other,
    AllOk,
}

static DISK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^Disk:\s*([0-9]+(?:\.[0-9]+)?)%\s*used\s*\(\s*threshold:\s*([0-9]+(?:\.[0-9]+)?)%\s*,\s*available:\s*(.+?)\s*/\s*total:\s*(.+?)\s*\)$",
    )
    .expect("disk metric regex")
});

static MEMORY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^Memory:\s*([0-9]+(?:\.[0-9]+)?)%\s*used\s*\(\s*threshold:\s*([0-9]+(?:\.[0-9]+)?)%\s*,\s*available:\s*([0-9]+(?:\.[0-9]+)?)\s*MB\s*/\s*total:\s*([0-9]+(?:\.[0-9]+)?)\s*MB\s*\)$",
    )
    .expect("memory metric regex")
});

static CPU_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^CPU load:\s*1m=([0-9]+(?:\.[0-9]+)?)\s*,\s*5m=([0-9]+(?:\.[0-9]+)?)\s*,\s*15m=([0-9]+(?:\.[0-9]+)?)\s*\(\s*limit:\s*([0-9]+(?:\.[0-9]+)?)\s*,\s*(\d+)\s*CPUs?\s*\)$",
    )
    .expect("cpu metric regex")
});

fn parse_server_payload(payload: &str) -> ParsedServer {
    let mut parsed = ParsedServer::default();
    let mut section = Section::None;
    for raw in payload.lines() {
        let line = strip_line_markers(raw.trim());
        if line.is_empty() {
            continue;
        }
        if let Some((next, remainder)) = detect_section(&line) {
            section = next;
            if !remainder.is_empty() {
                consume_content(&mut parsed, section, remainder);
            }
            continue;
        }
        consume_content(&mut parsed, section, line);
    }
    parsed
}

fn strip_line_markers(line: &str) -> String {
    let mut s = line;
    if let Some(rest) = s.strip_prefix("[STILL ONGOING]") {
        s = rest.trim_start();
    }
    if let Some(rest) = s.strip_prefix('\u{2705}') {
        s = rest.trim_start();
    }
    if let Some(rest) = s.strip_prefix("[RESOLVED]") {
        s = rest.trim_start();
    }
    s.to_string()
}

fn detect_section(line: &str) -> Option<(Section, String)> {
    const HEADERS: &[(&str, Section)] = &[
        ("Action needed:", Section::Action),
        ("Suggested next steps:", Section::Steps),
        ("Other metrics:", Section::Other),
        ("All metrics within thresholds:", Section::AllOk),
    ];
    for (header, section) in HEADERS {
        if let Some(idx) = line.find(header) {
            let remainder = line[idx + header.len()..].trim().to_string();
            return Some((*section, remainder));
        }
    }
    None
}

fn consume_content(parsed: &mut ParsedServer, section: Section, line: String) {
    if section == Section::Steps {
        if is_mapped_suggestion(&line) {
            return;
        }
        parsed.unparsed.push(line);
        return;
    }
    if let Some(metric) = parse_metric(&line) {
        match section {
            Section::Other => parsed.other.push(metric),
            Section::AllOk => parsed.all_ok.push(metric),
            Section::Action | Section::None | Section::Steps => parsed.action.push(metric),
        }
    } else {
        parsed.unparsed.push(line);
    }
}

fn is_mapped_suggestion(line: &str) -> bool {
    line.starts_with("Disk:") || line.starts_with("Memory:") || line.starts_with("CPU:")
}

fn parse_metric(line: &str) -> Option<Metric> {
    if let Some(caps) = DISK_RE.captures(line) {
        return Some(Metric::Disk {
            percent: cap_f64(&caps, 1)?,
            threshold: cap_f64(&caps, 2)?,
            available: caps.get(3)?.as_str().trim().to_string(),
            total: caps.get(4)?.as_str().trim().to_string(),
        });
    }
    if let Some(caps) = MEMORY_RE.captures(line) {
        return Some(Metric::Memory {
            percent: cap_f64(&caps, 1)?,
            threshold: cap_f64(&caps, 2)?,
            available_mb: cap_f64(&caps, 3)?,
            total_mb: cap_f64(&caps, 4)?,
        });
    }
    if let Some(caps) = CPU_RE.captures(line) {
        return Some(Metric::Cpu {
            load1: cap_f64(&caps, 1)?,
            load5: cap_f64(&caps, 2)?,
            load15: cap_f64(&caps, 3)?,
            limit: cap_f64(&caps, 4)?,
            cpus: caps.get(5)?.as_str().parse().ok()?,
        });
    }
    None
}

fn cap_f64(caps: &regex::Captures<'_>, i: usize) -> Option<f64> {
    caps.get(i)?.as_str().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENSHOT_PAYLOAD: &str = "\
[STILL ONGOING] Action needed:
Disk: 93.0% used (threshold: 70%, available: 13.6 GB / total: 192.7 GB)

Suggested next steps:
Disk: inspect large logs/build artifacts before deleting anything; prefer reversible cleanup.

Other metrics:
CPU load: 1m=1.63, 5m=1.52, 15m=1.57 (limit: 12.0, 4 CPUs)
Memory: 17.7% used (threshold: 80%, available: 19728 MB / total: 23974 MB)
";

    const ZH_SCREENSHOT_TITLE: &str = "服务器：磁盘快满了（还没解决）";
    const ZH_SCREENSHOT_BODY: &str = "磁盘用了 93%，只剩 13.6 GB（共 192.7 GB），超过 70% 的线。\n建议先看大日志和构建产物，能恢复的清理优先，别直接删。\n其它正常：CPU 负载 1.63 / 1.52 / 1.57（4 核，上限 12），内存用了 18%（剩 19.7 GB）。";

    const EN_SCREENSHOT_TITLE: &str = "Server: disk almost full (still unresolved)";
    const EN_SCREENSHOT_BODY: &str = "Disk at 93%, 13.6 GB left of 192.7 GB, over the 70% line.\nCheck large logs and build artifacts first; prefer reversible cleanup, do not just delete.\nOtherwise fine: CPU load 1.63 / 1.52 / 1.57 (4 cores, limit 12), memory at 18% (19.7 GB left).";

    fn new_disk_payload() -> String {
        SCREENSHOT_PAYLOAD.replacen("[STILL ONGOING] ", "", 1)
    }

    #[test]
    fn disk_pressure_is_detected_only_for_active_server_monitor_alerts() {
        assert!(payload_has_disk_pressure(
            "server_monitor",
            SCREENSHOT_PAYLOAD
        ));
        assert!(payload_has_disk_pressure(
            "skill_server_monitor",
            &new_disk_payload()
        ));
        assert!(!payload_has_disk_pressure(
            "server_monitor",
            &format!("[RESOLVED] {}", new_disk_payload())
        ));
        assert!(!payload_has_disk_pressure(
            "commute_traffic_monitor",
            SCREENSHOT_PAYLOAD
        ));
        let calm = "All metrics OK:\nDisk: 41.0% used (threshold: 70%, available: 110.0 GB / total: 192.7 GB)\n";
        assert!(!payload_has_disk_pressure("server_monitor", calm));
        // Rounded display can equal the threshold while the skill already put
        // the metric under "Action needed": still pressure.
        let at_line = "Action needed:\nDisk: 70.0% used (threshold: 70%, available: 57.2 GB / total: 192.7 GB)\n";
        assert!(payload_has_disk_pressure("server_monitor", at_line));
        assert!(!payload_has_disk_pressure(
            "server_monitor",
            "Error: probe failed"
        ));
    }

    #[test]
    fn screenshot_payload_zh_matches_exactly() {
        let alert = localize_heartbeat_alert("server_monitor", SCREENSHOT_PAYLOAD, "zh");
        assert_eq!(alert.state, AlertState::Ongoing);
        assert_eq!(alert.title, ZH_SCREENSHOT_TITLE);
        assert_eq!(alert.body, ZH_SCREENSHOT_BODY);
    }

    #[test]
    fn screenshot_payload_en_matches_exactly() {
        let alert = localize_heartbeat_alert("server_monitor", SCREENSHOT_PAYLOAD, "en");
        assert_eq!(alert.state, AlertState::Ongoing);
        assert_eq!(alert.title, EN_SCREENSHOT_TITLE);
        assert_eq!(alert.body, EN_SCREENSHOT_BODY);
    }

    #[test]
    fn new_disk_alert_drops_ongoing_suffix() {
        let alert = localize_heartbeat_alert("server_monitor", &new_disk_payload(), "zh");
        assert_eq!(alert.state, AlertState::New);
        assert_eq!(alert.title, "服务器：磁盘快满了");
        assert!(alert.body.starts_with("磁盘用了 93%"));
        assert!(!alert.title.contains("还没解决"));
    }

    #[test]
    fn multiple_alerts_join_title_issues() {
        let payload = "\
Action needed:
Disk: 93.0% used (threshold: 70%, available: 13.6 GB / total: 192.7 GB)
Memory: 91.2% used (threshold: 80%, available: 900 MB / total: 16000 MB)

Suggested next steps:
Disk: inspect large logs/build artifacts before deleting anything; prefer reversible cleanup.
Memory: inspect top processes and restart only the affected service if pressure is sustained.
";
        let zh = localize_heartbeat_alert("server_monitor", payload, "zh");
        assert_eq!(zh.title, "服务器：磁盘快满了、内存吃紧");
        assert!(zh.body.contains("建议先看大日志和构建产物"));
        assert!(zh.body.contains("建议先看占内存最多的进程"));
        let en = localize_heartbeat_alert("server_monitor", payload, "en");
        assert_eq!(en.title, "Server: disk almost full, memory tight");
    }

    #[test]
    fn resolved_alert_uses_recovered_title() {
        let payload = "\
\u{2705} [RESOLVED] server_monitor — All metrics within thresholds:
CPU load: 1m=1.63, 5m=1.52, 15m=1.57 (limit: 12.0, 4 CPUs)
Memory: 17.7% used (threshold: 80%, available: 19728 MB / total: 23974 MB)
Disk: 50.0% used (threshold: 70%, available: 96.3 GB / total: 192.7 GB)
";
        let alert = localize_heartbeat_alert("server_monitor", payload, "zh");
        assert_eq!(alert.state, AlertState::Resolved);
        assert_eq!(alert.title, "服务器：已恢复正常");
        assert_eq!(
            alert.body,
            "CPU 负载 1.63 / 1.52 / 1.57（4 核，上限 12），内存用了 18%（剩 19.7 GB），磁盘用了 50%（剩 96.3 GB）。"
        );
    }

    #[test]
    fn error_alert_keeps_verbatim_body() {
        let alert = localize_heartbeat_alert(
            "server_monitor",
            "Error: metrics endpoint returned 503",
            "zh",
        );
        assert_eq!(alert.state, AlertState::Error);
        assert_eq!(alert.title, "服务器检查没跑成");
        assert_eq!(alert.body, "metrics endpoint returned 503");
    }

    #[test]
    fn unknown_skill_uses_humanized_name_and_strips_markers() {
        let alert = localize_heartbeat_alert(
            "weather_watch",
            "[STILL ONGOING] rain expected overnight",
            "zh",
        );
        assert_eq!(alert.state, AlertState::Ongoing);
        assert_eq!(alert.title, "「Weather watch」提醒（还没解决）");
        assert_eq!(alert.body, "rain expected overnight");
        let named = localize_heartbeat_alert("calendar", "next event in 10 minutes", "zh");
        assert_eq!(named.title, "「日历」提醒");
        assert_eq!(named.body, "next event in 10 minutes");
    }

    #[test]
    fn mb_to_gb_rounding_and_small_mb_kept() {
        assert_eq!(format_mb(19728.0), "19.7 GB");
        assert_eq!(format_mb(512.0), "512 MB");
        let payload = "\
Action needed:
Memory: 91.0% used (threshold: 80%, available: 512 MB / total: 900 MB)
";
        let alert = localize_heartbeat_alert("server_monitor", payload, "zh");
        assert!(alert.body.contains("剩 512 MB"));
        assert!(alert.body.contains("共 900 MB"));
        assert!(!alert.body.contains("GB"));
    }

    #[test]
    fn unparseable_line_is_preserved() {
        let payload = "\
Action needed:
Disk: 93.0% used (threshold: 70%, available: 13.6 GB / total: 192.7 GB)
Kernel: dmesg overflow on nvme0

Suggested next steps:
Disk: inspect large logs/build artifacts before deleting anything; prefer reversible cleanup.
";
        let alert = localize_heartbeat_alert("server_monitor", payload, "zh");
        assert!(
            alert.body.contains("Kernel: dmesg overflow on nvme0"),
            "unparseable line must be kept: {}",
            alert.body
        );
        assert!(alert.body.contains("磁盘用了 93%"));
    }

    #[test]
    fn zh_copy_has_no_banned_jargon_or_english_sentences() {
        let samples = [
            localize_heartbeat_alert("server_monitor", SCREENSHOT_PAYLOAD, "zh"),
            localize_heartbeat_alert("server_monitor", &new_disk_payload(), "zh"),
            localize_heartbeat_alert(
                "server_monitor",
                "Error: metrics endpoint returned 503",
                "zh",
            ),
        ];
        let banned = ["回执", "事项", "主人", "幕僚长"];
        for alert in samples {
            let text = format!("{}\n{}", alert.title, alert.body);
            for word in banned {
                assert!(
                    !text.contains(word),
                    "banned {word} in zh alert copy: {text}"
                );
            }
            if alert.state != AlertState::Error {
                assert!(
                    !has_ascii_four_word_sentence(&alert.title)
                        && !has_ascii_four_word_sentence(&alert.body),
                    "zh alert copy contains a 4-word ASCII sentence:\n{text}"
                );
            }
        }
        let keys = [
            "alert.heartbeat.ongoing_suffix",
            "alert.heartbeat.server.prefix",
            "alert.heartbeat.server.generic",
            "alert.heartbeat.server.title_resolved",
            "alert.heartbeat.server.title_error",
            "alert.heartbeat.issue.disk",
            "alert.heartbeat.issue.memory",
            "alert.heartbeat.issue.cpu",
            "alert.heartbeat.disk.action",
            "alert.heartbeat.memory.action",
            "alert.heartbeat.cpu.action",
            "alert.heartbeat.suggest.disk",
            "alert.heartbeat.suggest.memory",
            "alert.heartbeat.suggest.cpu",
            "alert.heartbeat.other.sentence",
            "alert.heartbeat.disk.ok",
            "alert.heartbeat.memory.ok",
            "alert.heartbeat.cpu.ok",
            "alert.heartbeat.unknown.title",
            "alert.heartbeat.unknown.title_resolved",
            "alert.heartbeat.unknown.title_error",
        ];
        for key in keys {
            let zh = t(key, "zh");
            assert_ne!(zh, "[MISSING]", "missing zh key {key}");
            for word in banned {
                assert!(!zh.contains(word), "{key} contains banned {word}: {zh}");
            }
            assert!(
                !has_ascii_four_word_sentence(zh),
                "{key} zh has a 4-word ASCII sentence: {zh}"
            );
        }
    }

    fn has_ascii_four_word_sentence(text: &str) -> bool {
        let mut count = 0usize;
        for word in text.split_whitespace() {
            let core: String = word
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '\'' || *c == '-')
                .collect();
            let is_ascii_word = !core.is_empty()
                && core
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '\'' || c == '-')
                && core.chars().any(|c| c.is_ascii_alphabetic());
            if is_ascii_word {
                count += 1;
                if count >= 4 {
                    return true;
                }
            } else {
                count = 0;
            }
        }
        false
    }

    #[test]
    fn alert_state_tokens() {
        assert_eq!(alert_state("[RESOLVED] ok"), AlertState::Resolved);
        assert_eq!(alert_state("[STILL ONGOING] still"), AlertState::Ongoing);
        assert_eq!(alert_state("Error: boom"), AlertState::Error);
        assert_eq!(alert_state("Action needed:"), AlertState::New);
    }
}

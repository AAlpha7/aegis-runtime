//! Internationalization — all user-facing strings centralized here.
//! Usage: `t("key", locale)` returns the localized string.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Supported locales
pub const LOCALE_ZH: &str = "zh";
pub const LOCALE_EN: &str = "en";
pub const DEFAULT_LOCALE: &str = "zh";

/// Translation table: key -> { locale -> text }
static TRANSLATIONS: LazyLock<HashMap<&'static str, HashMap<&'static str, &'static str>>> =
    LazyLock::new(|| {
        let mut m: HashMap<&str, HashMap<&str, &str>> = HashMap::new();

        // Helper macro
        macro_rules! tr {
            ($key:expr, $zh:expr, $en:expr) => {
                let mut entry = HashMap::new();
                entry.insert("zh", $zh);
                entry.insert("en", $en);
                m.insert($key, entry);
            };
        }

        // ── Admin Panel Tabs ──
        tr!("tab.skills", "技能状态", "Skills");
        tr!("tab.intents", "定时任务", "Scheduled Tasks");
        tr!("tab.approvals", "待审批", "Pending Approvals");
        tr!("tab.review", "审查统计", "Review Guard");

        // ── Skills Tab ──
        tr!("col.name", "名称", "Name");
        tr!("col.status", "状态", "Status");
        tr!("col.calls", "调用", "Calls");
        tr!("col.success_rate", "成功率", "Success Rate");
        tr!("col.latency", "平均延迟", "Avg Latency");
        tr!("col.last_error", "最后错误", "Last Error");
        tr!("col.actions", "操作", "Actions");

        tr!("col.skill_name", "技能名", "Skill Name");
        tr!("col.llm_calls", "AI调用次数", "AI Calls");
        tr!("col.created_at", "创建时间", "Created");

        // ── Intents Tab ──
        tr!("intent.active", "活跃任务", "Active Tasks");
        tr!("intent.timed", "定时任务", "Timed Tasks");
        tr!("intent.event", "事件任务", "Event Tasks");
        tr!("intent.recurring", "循环任务", "Recurring Tasks");
        tr!("intent.monitoring", "监控任务", "Monitoring Tasks");
        tr!("col.type", "类型", "Type");
        tr!("col.layer", "触发层级", "Layer");
        tr!("priority.urgent", "紧急", "Urgent");
        tr!("priority.high", "高", "High");
        tr!("priority.normal", "普通", "Normal");
        tr!("priority.low", "低", "Low");
        tr!("task.count_suffix", "项任务", "task(s)");
        tr!("col.group_key", "分组标识", "Group Key");
        tr!("col.mode", "模式", "Mode");
        tr!("col.next_fire", "下次触发", "Next Fire");
        tr!("col.fires", "已触发", "Fired");
        tr!("col.max_fires", "上限", "Max");

        // ── Approvals Tab ──
        tr!("approval.pending", "项待处理", "items pending");
        tr!("approval.none", "暂无待审批项", "No pending approvals");
        tr!("col.content", "工具/内容", "Tool/Content");

        // ── Review Guard Tab ──
        tr!(
            "review.explanation",
            "审查管线会自动检查AI回复的真实性。当AI声称完成了某个操作但实际没有调用对应工具时，系统会拦截并修正回复。",
            "The review pipeline automatically checks AI replies for truthfulness. When the AI claims to have completed an action without actually calling the corresponding tool, the system intercepts and corrects the reply."
        );
        tr!("review.total", "过去24小时审查次数", "Reviews (24h)");
        tr!("review.saved", "智能节省率", "LLM Calls Saved");
        tr!(
            "review.saved_desc",
            "避免了多少次AI审查调用",
            "AI review calls avoided"
        );
        tr!("review.flagged", "拦截的虚假声明数", "False Claims Caught");
        tr!("review.latency", "平均审查耗时", "Avg Review Latency");
        tr!("review.no_data", "暂无数据", "No data yet");
        tr!("layer.fast_pass", "快速通过", "Fast Pass");
        tr!("layer.no_signal", "无完成信号", "No Signal");
        tr!("layer.sem_pass", "语义通过", "Semantic Pass");
        tr!("layer.sem_flag", "语义标记", "Semantic Flag");
        tr!("layer.llm", "AI复核", "LLM Review");
        tr!(
            "review.chart_label",
            "各审查层级处理分布",
            "Layer Distribution"
        );
        tr!(
            "review.flagged_title",
            "近期被拦截的回复",
            "Recently Flagged Replies"
        );
        tr!(
            "review.flagged_subtitle",
            "以下回复因涉嫌虚假声明被系统标记",
            "These replies were flagged for suspected false claims"
        );
        tr!("col.time", "时间", "Time");
        tr!("col.tool_matched", "涉及工具", "Tool Matched");
        tr!("col.similarity", "匹配度", "Similarity");
        tr!("col.reply_preview", "被拦截的回复内容", "Flagged Reply");
        tr!(
            "review.none_flagged",
            "过去24小时无拦截记录",
            "No flagged replies in 24h"
        );

        // ── Buttons ──
        tr!("btn.approve", "审批", "Approve");
        tr!("btn.cancel", "取消", "Cancel");
        tr!("btn.retry", "重试", "Retry");
        tr!("btn.delete", "删除", "Delete");
        tr!("btn.deactivate", "停用", "Deactivate");
        tr!("btn.activate", "激活", "Activate");
        tr!("btn.reset_breaker", "重置熔断", "Reset Breaker");
        tr!("btn.reject", "拒绝", "Reject");

        // ── Common ──
        tr!("common.error", "错误", "Error");
        tr!("common.success", "成功", "Success");
        tr!("common.loading", "加载中", "Loading");
        tr!("common.none", "无", "None");

        // ── Monitor Panel ──
        tr!("monitor.overview", "概览", "Overview");
        tr!("monitor.success_rate", "成功率", "Success Rate");
        tr!("monitor.avg_latency", "平均延迟", "Avg Latency");
        tr!(
            "monitor.circuit_alerts",
            "熔断告警",
            "Circuit Breaker Alerts"
        );
        tr!("monitor.urgent", "紧急事项", "Urgent Items");
        tr!("monitor.col_skill", "技能", "Skill");
        tr!("monitor.col_calls", "调用", "Calls");
        tr!("monitor.col_errors", "错误", "Errors");
        tr!("monitor.col_latency", "延迟", "Latency");
        tr!("monitor.col_circuit", "熔断器", "Circuit Breaker");
        tr!("monitor.circuit_open", "已熔断", "Tripped");
        tr!("monitor.circuit_ok", "正常", "OK");

        // ── Meta-Panel ──
        tr!("meta.title", "智能面板", "Smart Panels");
        tr!("meta.panel_count", "个面板已配置", "panel(s) configured");
        tr!("meta.group_alerts", "告警", "Alerts");
        tr!("meta.group_data", "数据源", "Data Feeds");
        tr!("meta.group_system", "系统", "System");
        tr!("meta.group_custom", "自定义", "Custom");
        tr!("meta.col_command", "命令", "Command");
        tr!("meta.col_title", "标题", "Title");
        tr!("meta.col_type", "类型", "Type");
        tr!("meta.col_status", "状态", "Status");
        tr!("meta.col_enabled", "启用", "Enabled");
        tr!("meta.col_last_used", "上次使用", "Last Used");
        tr!("meta.status_active", "活跃", "Active");
        tr!("meta.status_inactive", "未激活", "Inactive");
        tr!(
            "meta.tip",
            "提示：可以通过面板上的按钮来置顶或锁定面板。",
            "Tip: Use the buttons on each panel to pin or lock them."
        );
        tr!("meta.last_triggered", "上次触发", "Last triggered");
        tr!(
            "meta.feedback_label",
            "反馈意见 — 描述您想要的修改",
            "Feedback — describe changes you want"
        );
        tr!(
            "meta.feedback_placeholder",
            "例如：添加图表、调整布局、显示不同数据...",
            "E.g.: add charts, adjust layout, show different data..."
        );
        tr!("meta.submit_feedback", "提交反馈", "Submit Feedback");
        tr!("meta.pin", "置顶", "Pin");
        tr!("meta.unpin", "取消置顶", "Unpin");
        tr!("meta.lock", "锁定", "Lock");
        tr!("meta.unlock", "解锁", "Unlock");

        // ── Panel Type Display Names ──
        tr!("panel_type.action_queue", "任务队列", "Task Queue");
        tr!("panel_type.data_feed", "数据源", "Data Feed");
        tr!("panel_type.monitor", "监控面板", "Monitor");

        tr!("panel_type.market", "市场行情", "Market");
        tr!("panel_type.alert", "告警中心", "Alert Center");
        tr!("panel_type.system_admin", "系统管理", "System Admin");
        tr!("panel_type.custom", "自定义面板", "Custom Panel");
        tr!("panel_type.heartbeat_audit", "心跳审计", "Heartbeat Audit");
        tr!("hb.total_cycles", "总周期", "Total Cycles");
        tr!("hb.heartbeat_cycles", "真心跳周期", "Heartbeat Cycles");
        tr!("hb.probe_cycles", "探针周期", "Probe Cycles");
        tr!("hb.total_tokens", "总Token", "Total Tokens");
        tr!("hb.heartbeat_tokens", "心跳Token", "Heartbeat Tokens");
        tr!("hb.probe_tokens", "探针Token", "Probe Tokens");
        tr!("hb.tab_all", "全部", "All");
        tr!("hb.tab_heartbeat", "真心跳", "Heartbeat");
        tr!("hb.tab_probe", "探针", "Probe");
        tr!("hb.no_entries", "暂无记录", "No entries");
        tr!("hb.true_heartbeat", "真心跳", "True Heartbeat");
        tr!("hb.probe", "探针", "Probe");
        tr!("hb.type", "类型", "Type");
        tr!("hb.time", "时间", "Time");
        tr!("hb.tokens", "Token", "Tokens");
        tr!("hb.rounds", "轮数", "Rounds");
        tr!("hb.severity", "严重性", "Severity");
        tr!("hb.tools_called", "工具调用", "Tools Called");
        tr!("hb.actions", "动作", "Actions");
        tr!("hb.llm_reply", "LLM回复", "LLM Reply");
        tr!("hb.raw_process", "原始过程", "Raw Process");
        tr!(
            "hb.trigger_btn",
            "⚡ 手动触发心跳 Manual Trigger",
            "⚡ Manual Trigger"
        );

        // ── ActionQueue / Tasks ──
        tr!("task.no_active", "暂无活跃任务。", "No active tasks.");
        tr!(
            "task.guide",
            "发送\"提醒我...\"或\"添加任务...\"来创建您的第一个任务。",
            "Send \"remind me to...\" or \"add task...\" to create your first task."
        );
        tr!("task.todo", "待办", "To-Do");
        tr!("task.reminder", "提醒", "Reminders");
        tr!("task.recurring", "循环", "Recurring");
        tr!("task.system", "系统", "System");
        tr!("task.completed", "已完成", "Completed");
        tr!("task.scheduled", "计划任务", "Scheduled");
        tr!("task.btn_done", "完成", "Done");
        tr!("task.btn_skip", "跳过一次", "Skip Once");
        tr!("task.btn_stop", "停止", "Stop");
        tr!("task.btn_archive", "归档", "Archive");
        tr!("task.btn_restore", "恢复", "Restore");
        tr!("task.btn_delete", "删除", "Delete");
        tr!("task.btn_clear", "清空全部", "Clear All");
        tr!("task.status_archived", "已归档", "Archived");
        tr!("task.status_completed", "已完成", "Completed");
        tr!("task.status_fired", "已触发", "Triggered");
        tr!("task.status_cancelled", "已取消", "Cancelled");
        tr!("task.priority", "优先级", "Priority");
        tr!("task.due", "截止", "Due");
        tr!("task.trigger", "触发", "Trigger");
        tr!("task.every", "每隔", "Every");
        tr!("task.items", "项", "items");

        // ── DataFeed ──
        tr!("feed.records", "条记录", "records");
        tr!("feed.category", "分类", "category");
        tr!("feed.col_title", "标题", "Title");
        tr!("feed.col_status", "状态", "Status");
        tr!("feed.col_updated", "更新时间", "Updated");

        // ── Market ──
        tr!("market.no_data", "暂无数据", "No data");

        // ── Alert Panel ──
        tr!("alert.title", "告警中心", "Alert Center");
        tr!(
            "alert.circuit_section",
            "熔断器告警",
            "Circuit Breaker Alerts"
        );
        tr!("alert.urgent_section", "紧急事项", "Urgent Items");
        tr!(
            "alert.heartbeat_section",
            "心跳检测告警",
            "Heartbeat Alerts"
        );
        tr!("alert.past_24h", "过去24小时", "last 24h");
        tr!("alert.monitoring_section", "活跃监控", "Active Monitoring");
        tr!(
            "alert.all_clear",
            "系统运行正常，无告警。",
            "All systems normal, no alerts."
        );
        tr!(
            "alert.skills_healthy",
            "个技能健康运行",
            "skills running healthy"
        );
        tr!("alert.last_check", "上次检查", "Last check");
        tr!(
            "alert.consecutive_failures",
            "连续失败",
            "consecutive failures"
        );
        tr!("alert.last_error", "最近错误", "Last error");
        tr!("alert.unknown", "未知", "Unknown");

        // ── Heartbeat owner-facing alert copy (Channel B) ──
        tr!(
            "alert.heartbeat.ongoing_suffix",
            "（还没解决）",
            " (still unresolved)"
        );
        tr!("alert.heartbeat.join", "、", ", ");
        tr!(
            "alert.heartbeat.disk.largest",
            "占用最多的目录：",
            "Largest directories:"
        );
        tr!("alert.heartbeat.clause_join", "，", ", ");
        tr!("alert.heartbeat.server.prefix", "服务器：", "Server: ");
        tr!(
            "alert.heartbeat.server.generic",
            "服务器提醒",
            "Server alert"
        );
        tr!(
            "alert.heartbeat.server.title_resolved",
            "服务器：已恢复正常",
            "Server: back to normal"
        );
        tr!(
            "alert.heartbeat.server.title_error",
            "服务器检查没跑成",
            "Server check didn't run"
        );
        tr!(
            "alert.heartbeat.issue.disk",
            "磁盘快满了",
            "disk almost full"
        );
        tr!("alert.heartbeat.issue.memory", "内存吃紧", "memory tight");
        tr!("alert.heartbeat.issue.cpu", "CPU 过载", "CPU overloaded");
        tr!(
            "alert.heartbeat.disk.action",
            "磁盘用了 {0}%，只剩 {1}（共 {2}），超过 {3}% 的线。",
            "Disk at {0}%, {1} left of {2}, over the {3}% line."
        );
        tr!(
            "alert.heartbeat.memory.action",
            "内存用了 {0}%，只剩 {1}（共 {2}），超过 {3}% 的线。",
            "Memory at {0}%, {1} left of {2}, over the {3}% line."
        );
        tr!(
            "alert.heartbeat.cpu.action",
            "CPU 负载 {0} / {1} / {2}（{3} 核，上限 {4}），已经超过线。",
            "CPU load {0} / {1} / {2} ({3} cores, limit {4}), over the line."
        );
        tr!(
            "alert.heartbeat.suggest.disk",
            "建议先看大日志和构建产物，能恢复的清理优先，别直接删。",
            "Check large logs and build artifacts first; prefer reversible cleanup, do not just delete."
        );
        tr!(
            "alert.heartbeat.suggest.memory",
            "建议先看占内存最多的进程，只在持续吃紧时重启相关服务。",
            "Check the processes using the most memory; restart the related service only if pressure stays high."
        );
        tr!(
            "alert.heartbeat.suggest.cpu",
            "建议先看最近的任务和服务日志，再考虑扩容或重启。",
            "Check recent jobs and service logs before scaling or restarting."
        );
        tr!(
            "alert.heartbeat.other.sentence",
            "其它正常：{0}。",
            "Otherwise fine: {0}."
        );
        tr!("alert.heartbeat.resolved.sentence", "{0}。", "{0}.");
        tr!(
            "alert.heartbeat.disk.ok",
            "磁盘用了 {0}%（剩 {1}）",
            "disk at {0}% ({1} left)"
        );
        tr!(
            "alert.heartbeat.memory.ok",
            "内存用了 {0}%（剩 {1}）",
            "memory at {0}% ({1} left)"
        );
        tr!(
            "alert.heartbeat.cpu.ok",
            "CPU 负载 {0} / {1} / {2}（{3} 核，上限 {4}）",
            "CPU load {0} / {1} / {2} ({3} cores, limit {4})"
        );
        tr!("alert.heartbeat.unknown.title", "「{0}」提醒", "{0} alert");
        tr!(
            "alert.heartbeat.unknown.title_resolved",
            "「{0}」已恢复正常",
            "{0}: back to normal"
        );
        tr!(
            "alert.heartbeat.unknown.title_error",
            "「{0}」没跑成",
            "{0} didn't run"
        );

        // ── Dashboard ──
        tr!("dash.title", "仪表盘", "Dashboard");
        tr!("dash.status", "状态概览", "Status Summary");
        tr!("dash.active_skills", "活跃技能", "Active Skills");
        tr!("dash.pending_alerts", "待处理告警", "Pending Alerts");
        tr!("dash.upcoming_tasks", "待办任务", "Upcoming Tasks");
        tr!("dash.pending_approvals", "待审批", "Pending Approvals");
        tr!("dash.recent_alerts", "最近告警", "Recent Alerts");
        tr!(
            "dash.no_alerts",
            "一切正常，无活跃告警",
            "All systems normal, no active alerts"
        );
        tr!("dash.health", "系统健康", "System Health");
        tr!("dash.success_rate", "整体成功率", "Overall Success Rate");
        tr!("dash.circuit_open", "熔断器已触发", "Circuit breaker OPEN");
        tr!("dash.col_skill", "技能", "Skill");
        tr!("dash.col_rate", "成功率", "Success %");
        tr!("dash.col_status", "状态", "Status");
        tr!("dash.status_open", "已熔断", "OPEN");
        tr!("dash.status_ok", "正常", "OK");
        tr!("dash.last_updated", "最后更新", "Last updated");
        tr!("dash.quick_links", "快捷入口", "Quick Links");

        // ── Intent Tab Additions ──
        tr!("intent.type_timed", "定时", "Timed");
        tr!("intent.type_event", "事件", "Event");
        tr!("intent.type_recurring", "循环", "Recurring");
        tr!("intent.type_monitoring", "监控", "Monitor");
        tr!("intent.mode_llm", "AI回调", "AI Callback");
        tr!("intent.mode_direct", "直接执行", "Direct");
        tr!("intent.view_detail", "查看详情", "View Details");
        tr!("intent.minutes_later", "分后", "m later");
        tr!("intent.hours_later", "小时后", "h later");
        tr!("intent.days_later", "天后", "d later");
        tr!("intent.expired", "已过期", "Expired");
        tr!("intent.on_event", "事件", "On event");
        tr!("intent.no_active", "暂无活跃任务", "No active tasks");

        // ── Common (additional) ──
        tr!("common.no_data", "暂无数据", "No data");
        tr!("common.total", "共", "Total");

        // ── Skills Tab (status strings) ──
        tr!("skill.status_active", "正常", "Active");
        tr!("skill.status_degraded", "降级", "Degraded");
        tr!("skill.status_tripped", "熔断", "Tripped");
        tr!("skill.status_deactivated", "停用", "Deactivated");
        tr!(
            "skill.circuit_alert",
            "熔断器已触发",
            "Circuit breaker tripped"
        );
        tr!("skill.chart_calls", "调用次数", "Call Count");

        // ── Telegram Messages ──
        tr!(
            "telegram.message_too_long",
            "⚠️ 消息过长，请缩短后重试（上限 64KB）。",
            "⚠️ Message too long. Please shorten and retry (limit 64KB)."
        );
        tr!(
            "telegram.audio_not_supported",
            "ℹ️ 当前模型不支持语音输入，已通过语音转写处理。",
            "ℹ️ Current model does not support voice input. Processed via speech-to-text."
        );
        tr!(
            "telegram.audio_stt_failed",
            "⚠️ 语音转写失败，请重新发送或改用文字输入。",
            "⚠️ Speech-to-text transcription failed. Please resend or use text input."
        );
        tr!(
            "telegram.audio_stt_not_configured",
            "⚠️ 当前模型不支持语音输入，且语音转写未配置。请改用文字或联系管理员配置 STT。",
            "⚠️ Current model does not support voice input and STT is not configured. Please use text or ask admin to configure STT."
        );
        tr!("telegram.quote_prefix", "[引用消息]", "[Quoted message]");
        tr!("telegram.reply_prefix", "[用户回复]", "[User reply]");
        tr!(
            "telegram.keyboard_placeholder",
            "输入消息或点击快捷按钮...",
            "Type a message or tap a button..."
        );
        tr!(
            "telegram.oauth_deeplink_success",
            "✅ OAuth 授权已完成！欢迎回来。\n\n你现在可以使用日历等功能了。",
            "✅ OAuth authorized! Welcome back.\n\nYou can now use calendar and other features."
        );
        // Reply keyboard button labels
        tr!("telegram.kb_status", "📊 系统状态", "📊 Status");
        tr!("telegram.kb_recall", "🧠 回忆", "🧠 Recall");
        tr!("telegram.kb_tasks", "📋 任务", "📋 Tasks");
        tr!("telegram.kb_search", "🔍 搜索", "🔍 Search");
        tr!("telegram.kb_settings", "⚙️ 设置", "⚙️ Settings");

        // ── Chronos ──
        tr!(
            "chronos.daily_summary_title",
            "📋 **每日系统摘要 / Daily System Summary**",
            "📋 **Daily System Summary**"
        );

        // ── Approvals Tab (gate buttons) ──
        tr!("approval.approve_once", "批准本次", "Approve Once");
        tr!("approval.approve_intent", "批准此Intent", "Approve Intent");
        tr!("approval.trust_domain", "信任域名", "Trust Domain");
        tr!("approval.allow_always", "永久允许", "Allow Always");

        // ── Chronos daily summary labels ──
        tr!(
            "chronos.heartbeat_memo_archive",
            "心跳便签归档",
            "Heartbeat memo archived"
        );
        tr!("chronos.daily_stats_header", "📊 过去24小时", "📊 Last 24h");
        tr!("chronos.tools_used", "🔧 工具使用", "🔧 Tools Used");
        tr!(
            "chronos.latest_heartbeat",
            "💬 最近心跳摘要",
            "💬 Latest Heartbeat"
        );
        tr!("chronos.current_memo", "📝 当前便签", "📝 Current Memo");
        tr!(
            "chronos.tool_effectiveness",
            "📈 工具效果",
            "📈 Tool Effectiveness"
        );
        tr!("chronos.wasteful_tools", "⚠️ 低效工具", "⚠️ Wasteful");
        tr!("chronos.budget", "💰 预算", "💰 Budget");

        // ── Callback Status Strings ──
        tr!("callback.retrying", "正在重试...", "Retrying...");
        tr!("callback.deleting", "正在删除...", "Deleting...");
        tr!("callback.deactivating", "正在停用...", "Deactivating...");
        tr!(
            "callback.processing_approval",
            "正在处理审批...",
            "Processing approval..."
        );
        tr!(
            "callback.approving_intent",
            "正在审批此 Intent...",
            "Approving for this intent..."
        );
        tr!(
            "callback.approving_domain",
            "正在审批此域名...",
            "Approving for this domain..."
        );
        tr!(
            "callback.approving_always",
            "正在永久审批...",
            "Permanently approving..."
        );
        tr!(
            "callback.requesting_clarification",
            "正在请求澄清...",
            "Requesting clarification..."
        );
        tr!("callback.deferring", "正在推迟...", "Deferring...");
        tr!("callback.dismissing", "正在忽略...", "Dismissing...");
        tr!(
            "callback.blocking",
            "正在永久封禁...",
            "Permanently blocking..."
        );
        tr!(
            "callback.recording_feedback",
            "正在记录反馈...",
            "Recording feedback..."
        );
        tr!(
            "callback.preparing_followup",
            "正在准备跟进...",
            "Preparing follow-up..."
        );
        tr!("callback.rejecting", "正在拒绝...", "Rejecting...");
        tr!("callback.approved", "✅ 已批准", "✅ APPROVED");
        tr!(
            "callback.approved_once",
            "✅ 已批准（本次）",
            "✅ APPROVED (once)"
        );
        tr!(
            "callback.approved_intent",
            "✅ 已批准（Intent）",
            "✅ APPROVED (intent)"
        );
        tr!(
            "callback.approved_domain",
            "✅ 已批准（域名）",
            "✅ APPROVED (domain)"
        );
        tr!(
            "callback.approved_always",
            "✅ 已批准（永久）",
            "✅ APPROVED (always)"
        );
        tr!("callback.deferred", "⏳ 已推迟", "⏳ DEFERRED");
        tr!("callback.dismissed", "🚫 已忽略", "🚫 DISMISSED");
        tr!("callback.blocked", "⛔ 已封禁", "⛔ BLOCKED");
        tr!("callback.rejected", "❌ 已拒绝", "❌ REJECTED");
        tr!("callback.retrying_status", "🔄 正在重试", "🔄 RETRYING");
        tr!("callback.deleted", "🗑 已删除", "🗑 DELETED");
        tr!("callback.helpful", "👍 有帮助", "👍 HELPFUL");
        tr!("callback.not_helpful", "👎 无帮助", "👎 NOT HELPFUL");
        tr!("callback.following_up", "📋 正在跟进", "📋 FOLLOWING UP");
        tr!("callback.deactivated", "🚫 已停用", "🚫 DEACTIVATED");
        tr!("callback.clarifying", "❓ 正在澄清", "❓ CLARIFYING");
        tr!("callback.cancelled", "🚫 已取消", "🚫 CANCELLED");
        tr!("callback.unknown_action", "未知操作。", "Unknown action.");
        tr!(
            "callback.already_processed",
            "已处理。",
            "Already processed."
        );
        tr!(
            "callback.already_processing",
            "⏳ 已在处理中…",
            "⏳ Already processing…"
        );
        tr!("callback.processing", "⏳ 处理中…", "⏳ Processing…");
        tr!(
            "callback.internal_error",
            "❌ 内部错误 — 已记录",
            "❌ Internal error — logged"
        );

        // ── Inline approval button labels ──
        // These are the labels that appear ON the Telegram inline keyboard
        // buttons (vs callback.* above, which is ACK text after a click).
        tr!("button.approve", "✅ 批准", "✅ Approve");
        tr!("button.reject", "❌ 拒绝", "❌ Reject");
        tr!("button.gate_once", "✅ 批准本次", "✅ Approve once");
        tr!("button.gate_intent", "🔄 批准此Intent", "🔄 Approve intent");
        // {} is the domain name (populated via `tf()`).
        tr!("button.gate_domain_trust", "🌐 信任 {}", "🌐 Trust {}");
        tr!("button.gate_always", "♾️ 永久允许", "♾️ Always allow");
        tr!("button.gate_reject", "❌ 拒绝", "❌ Reject");
        tr!("button.gate_clarify", "❓ 要求澄清", "❓ Ask to clarify");

        // ── Access / Error Strings ──
        tr!(
            "error.access_denied",
            "拒绝访问。此机器人为私有。",
            "Access denied. This bot is private."
        );
        tr!("error.no_data", "无数据", "No data");
        tr!(
            "error.llm_timeout",
            "⏱ LLM 响应超时，可能是模型服务繁忙。请稍后重试。",
            "⏱ LLM response timed out. The model service may be busy. Please retry later."
        );
        tr!(
            "error.rate_limit",
            "⚠️ API 请求频率受限，请稍等几秒再试。",
            "⚠️ API rate limited. Please wait a few seconds and retry."
        );
        // Edge-triggered owner alert when ALL LLM providers/fallbacks are
        // exhausted for a foreground request (sent via Telegram, not the LLM).
        // {0} = short reason (e.g. provider usage limit / upstream error head).
        tr!(
            "provider.degraded.telegram",
            "⚠️ LLM 服务不可用：{0}。聊天暂时无法回复，请在网页设置里切换 provider 或等待恢复。",
            "⚠️ LLM service unavailable: {0}. Chat cannot reply right now — switch provider in the web settings or wait for recovery."
        );
        // Localized owner-facing "reason" buckets for the degraded banner/alert
        // `{reason}` slot. Categorized from the terminal LLM error so the owner
        // never sees an English fragment inside the localized banner/Telegram
        // alert (the raw English error head is kept only in tracing logs).
        tr!(
            "provider.degraded.reason.usage_limit",
            "服务商用量/额度已耗尽",
            "Provider usage limit / credit exhausted"
        );
        tr!(
            "provider.degraded.reason.all_unavailable",
            "所有 LLM 服务商当前均不可用",
            "All LLM providers are currently unavailable"
        );
        tr!(
            "provider.degraded.reason.generic",
            "暂时无法连接 AI 服务商",
            "Temporarily unable to reach the AI provider"
        );
        // Backup-model drill (admin diagnostic). Owner-visible error text
        // is composed zh-first with the English string in parentheses.
        tr!(
            "diagnostics.fallback_drill.not_authorized",
            "备用模型未授权，主模型不可用时无法自动切换。",
            "The backup model is not authorized, so chat cannot switch automatically when the primary is unavailable."
        );
        tr!(
            "diagnostics.fallback_drill.no_backup",
            "没有配置可用的备用模型。",
            "No usable backup model is configured."
        );
        tr!(
            "diagnostics.fallback_drill.backup_failed",
            "备用模型未能完成回复。",
            "The backup model did not complete a reply."
        );
        tr!(
            "settings.audit.fallback_authorized_without_model",
            "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。",
            "Backup model is authorized, but no backup model is selected: chat will not switch automatically when the primary fails."
        );
        tr!(
            "settings.audit.fallback_model_equals_primary",
            "备用模型和主模型是同一个：主模型出问题时没有真正的备用。",
            "The backup model is the same as the primary model, so it cannot take over when the primary fails."
        );
        tr!(
            "settings.audit.provider_without_model",
            "已经选了语言模型服务商，但还没选模型：现在没法开始对话。",
            "A language provider is selected, but no model is set, so chat cannot start."
        );
        tr!(
            "settings.audit.paid_fallback_without_provider",
            "付费备用通道已打开，但还没有配置付费服务商：主路径出问题时没法切换。",
            "Paid fallback is on, but no paid provider credentials are configured, so it cannot take over."
        );
        tr!(
            "settings.audit.external_executor_disabled_but_google_routing",
            "外部执行层已关闭，Google 的数据现在读不到。",
            "The external coding agent is off, so Google data cannot be reached."
        );
        tr!(
            "settings.audit.llm_model_not_in_provider_catalog",
            "模型设置里的 {model} 不在 {provider} 的模型列表里，调用可能失败；建议在设置页重选。",
            "Model setting {model} is not in {provider}'s model list, so calls may fail; reselect it in Settings."
        );
        tr!(
            "settings.audit.fallback_model_not_in_provider_catalog",
            "备用模型设置里的 {model} 不在 {provider} 的模型列表里，切换时可能失败；建议在设置页重选。",
            "Backup model setting {model} is not in {provider}'s model list, so switching to it may fail; reselect it in Settings."
        );
        tr!(
            "settings.audit.next",
            "到设置里改。",
            "Fix this in Settings."
        );
        // WebApp contextual-assistant (Channel B, owner-facing direct display)
        // unavailable / empty-response messages. Rendered in display_language.
        tr!(
            "assistant.unavailable.body",
            "助手模型暂时不可用。未读取任何隐藏字段，也未执行任何操作。",
            "The assistant model is temporarily unavailable. No hidden fields were read and no action was taken."
        );
        tr!(
            "assistant.unavailable.warning",
            "助手模型不可用",
            "Assistant model is unavailable"
        );
        tr!(
            "assistant.empty_response",
            "助手返回了空回复。",
            "The assistant returned an empty response."
        );
        tr!(
            "assistant.turn_unanswered",
            "这轮没有生成回复。消息已留下，可以再发一次。",
            "This turn produced no reply. Your message is saved; send it again."
        );
        tr!(
            "assistant.turn_unanswered.with_reason",
            "这轮没有生成回复。消息已留下，可以再发一次。原因：{}",
            "This turn produced no reply. Your message is saved; send it again. Reason: {}"
        );
        tr!(
            "error.auth_failed",
            "🔑 API 认证失败，请联系管理员检查配置。",
            "🔑 API authentication failed. Please contact the admin to check config."
        );
        tr!(
            "error.connection_failed",
            "🌐 无法连接到 LLM 服务，请稍后重试。",
            "🌐 Cannot connect to LLM service. Please retry later."
        );
        tr!(
            "error.processing_error",
            "❗ 处理出错: {}。请重试，如反复出现请联系管理员。",
            "❗ Processing error: {}. Please retry. If it persists, contact the admin."
        );

        // ── HITL approval/rejection (direct to user via Telegram) ──
        tr!(
            "hitl.chat_mismatch",
            "审批 `{}` 来自另一个聊天窗口，拒绝访问。",
            "Approval `{}` was created in a different chat. Access denied."
        );
        tr!(
            "hitl.not_found_or_processed",
            "审批 `{}` 未找到或已处理。",
            "Approval `{}` not found or already processed."
        );
        tr!(
            "hitl.sysop_rejected",
            "管理员操作已拒绝。",
            "SysAdmin operation rejected."
        );
        tr!(
            "hitl.config_rejected",
            "配置变更 '{}' 已拒绝。",
            "Config change '{}' rejected."
        );
        tr!(
            "hitl.mcp_approved",
            "✅ MCP 服务器 `{}` 已批准，工具现在可用。",
            "✅ MCP server `{}` approved. Its tools are now available."
        );
        tr!(
            "hitl.mcp_rejected",
            "❌ MCP 服务器 `{}` 已拒绝，其工具不会使用。",
            "❌ MCP server `{}` rejected. Its tools will not be used."
        );
        tr!(
            "hitl.menu_approved",
            "✅ 菜单命令已执行。",
            "✅ Menu command executed."
        );
        tr!(
            "hitl.menu_rejected",
            "❌ 菜单命令已拒绝。",
            "❌ Menu command rejected."
        );
        tr!(
            "hitl.scope_fallback_intent_to_once",
            "⚠️ 你要求的 intent 级别批准因缺少 `intent_group_key` 已降级为一次性批准。",
            "⚠️ Intent-scoped approval was downgraded to one-shot (missing group_key in approval data)."
        );
        tr!(
            "hitl.scope_fallback_domain_to_once",
            "⚠️ 你要求的 domain 级别批准因缺少 `url_host` 已降级为一次性批准。",
            "⚠️ Domain-scoped approval was downgraded to one-shot (missing url_host in approval data)."
        );
        tr!(
            "hitl.approval_corrupted",
            "❌ 批准数据损坏，操作未执行。请重新发起。",
            "❌ Approval data corrupted; operation not executed. Please retry."
        );
        tr!(
            "hitl.config_gate_pre_approved",
            "✅ 已应用 — 网关已批准，配置直通生效。",
            "✅ Applied — gate approval covered this change; config landed without a second prompt."
        );

        // ── Config apply-result strings (Channel B — owner display_language) ──
        // Produced by `apply_approved_config`'s ConfigApplyResult::localized().
        // ALL interpolated args (setting key, old/new values, embedding
        // from/to/reason) are DATA-DERIVED / ARBITRARY → rendered via `tfp`
        // (single-pass, no re-scan). The Channel-A LLM callback uses the raw
        // English `.callback_string()` form instead and never touches these.
        // {0}=key {1}=value — BOTH ARBITRARY — use tfp.
        tr!(
            "config.exec.updated_to",
            "设置 '{0}' 已更新为 {1}。",
            "Setting '{0}' updated to {1}."
        );
        // {0}=key (ARBITRARY) — use tfp.
        tr!(
            "config.exec.unchanged",
            "设置 '{0}' 未变化（提交值与当前一致）。",
            "Setting '{0}' unchanged (payload matched current)."
        );
        // PREFIX-only via tfp: {0}=key (ARBITRARY); arbitrary error concatenated AFTER.
        tr!(
            "config.exec.apply_failed",
            "应用设置 '{0}' 失败：",
            "Failed to apply setting '{0}': "
        );
        // {0}=key (ARBITRARY) — use tfp. (Fixed-suffix variant of apply_failed.)
        tr!(
            "config.exec.invalid_payload",
            "应用设置 '{0}' 失败：JSON 负载无效",
            "Failed to apply setting '{0}': invalid JSON payload"
        );
        // {0}=key {1}=old {2}=new — ALL ARBITRARY — use tfp. Hot-reload note (if
        // any) is a separately-localized string concatenated AFTER this one.
        tr!(
            "config.exec.changed_from_to",
            "设置 '{0}' 已从 {1} 变更为 {2}。",
            "Setting '{0}' changed from {1} to {2}."
        );
        // {0}=from {1}=to — ARBITRARY — use tfp. Leading space is intentional
        // (concatenated after changed_from_to).
        tr!(
            "config.exec.hot_reloaded",
            " 已热重载（{0} → {1}）。",
            " Hot-reloaded ({0} → {1})."
        );
        // {0}=old {1}=new {2}=key — ALL ARBITRARY — use tfp. Leading space intentional.
        tr!(
            "config.exec.requires_reembed",
            " 已热重载（{0} → {1}）。\u{26A0}\u{FE0F}  现存的嵌入语料是用旧的 {2} \
             值生成的；与新值计算的余弦相似度不再有意义。请调用 \
             `trigger_corpus_reembed`（需用户批准）用新设置重新计算所有向量。",
            " Hot-reloaded ({0} → {1}). \u{26A0}\u{FE0F}  The stored embedding corpus was \
             produced by the old {2} value; cosine similarity against the new one is not \
             meaningful. Please call `trigger_corpus_reembed` (user-approved) to recompute \
             all vectors with the new setting."
        );
        // {0}=reason — ARBITRARY — use tfp. Leading space intentional.
        tr!(
            "config.exec.hot_reload_rejected",
            " \u{26A0}\u{FE0F}  热重载被拒绝：{0}（设置行已存储，但活动客户端仍使用旧值 \
             — 重启后会生效，或提交一个有效值）",
            " \u{26A0}\u{FE0F}  Hot-reload REJECTED: {0} (setting row stored, but live client \
             still uses previous value — a restart will pick up the setting, or propose a valid value)"
        );

        // Gate direct-execution result prefix (Channel B — no-LLM path renders
        // straight to the owner via format_gate_result). PREFIX-only: the
        // arbitrary tool error string is concatenated AFTER (never re-scanned).
        tr!("gate.result.error_prefix", "错误：", "Error: ");

        // ── HITL approval-result strings (Channel B — owner display_language) ──
        // Task 3: these were previously hardcoded English or pinned to
        // DEFAULT_LOCALE; they now resolve under the owner's display_language.
        // PREFIX-ONLY keys (an arbitrary error string is concatenated AFTER —
        // never routed through tf/tfn, which would re-scan a literal `{}` in the
        // error and corrupt it; see hitl_arbitrary_string_keys_are_prefix_only_no_tfn_rescan).

        // {} = approval id (safe identifier)
        tr!(
            "hitl.gate.not_found_or_expired",
            "审批 `{}` 未找到或已过期。",
            "Approval `{}` not found or expired."
        );

        // {} = approval id (safe identifier)
        tr!(
            "hitl.config.not_found_or_processed",
            "配置提案 `{}` 未找到或已处理。",
            "Config proposal `{}` not found or already processed."
        );
        // {} = approval id (safe identifier)
        tr!(
            "hitl.config.not_found_maybe_processed",
            "配置提案 `{}` 未找到（可能已处理）。",
            "Config proposal `{}` not found (may have already been processed)."
        );

        // {} = approval id (safe identifier)
        tr!(
            "hitl.revoke.not_found_or_processed",
            "撤销审批 `{}` 未找到或已处理。",
            "Revoke approval `{}` not found or already processed."
        );
        // {} = account name (identifier); PREFIX — error concatenated after
        tr!(
            "hitl.revoke.failed",
            "账户 `{}` 的 Google OAuth 撤销失败：",
            "Google OAuth revocation failed for account `{}`: "
        );
        // {} = account name (identifier)
        tr!(
            "hitl.revoke.success",
            "账户 `{}` 的 Google OAuth2 凭据已撤销并移除。",
            "Google OAuth2 credentials revoked and removed for account `{}`."
        );

        tr!(
            "hitl.mcp.invalid_missing_key",
            "无效的 MCP 审批（缺少 server_key）。",
            "Invalid MCP approval (missing server_key)."
        );

        // ── HITL sysop approval progress/result (Channel B — display_language) ──
        // {} = short job id
        tr!(
            "hitl.sysop.job_submitted",
            "⏳ 任务已提交 (job: {}), 正在执行...",
            "⏳ Job submitted (job: {}), executing..."
        );
        // {} = consecutive failure count
        tr!(
            "hitl.sysop.executor_unreachable",
            "❌ Executor 连接失败（连续 {} 次），可能已崩溃（OOM 等）。任务中止。",
            "❌ Executor unreachable ({} consecutive failures); it may have crashed (OOM, etc.). Job aborted."
        );
        // {0}=completed {1}=total {2}=elapsed_secs {3}=current command
        tr!(
            "hitl.sysop.progress_with_cmd",
            "⏳ [{0}/{1}] 执行中 ({2}s): ",
            "⏳ [{0}/{1}] running ({2}s): "
        );
        // {0}=completed {1}=total {2}=elapsed_secs
        tr!(
            "hitl.sysop.progress",
            "⏳ [{0}/{1}] 执行中 ({2}s)...",
            "⏳ [{0}/{1}] running ({2}s)..."
        );
        // {0}=status {1}=elapsed_secs
        tr!(
            "hitl.sysop.done",
            "✅ 执行完毕 (状态: {0}, {1}s)",
            "✅ Finished (status: {0}, {1}s)"
        );
        // {0}=tool name {1}=error detail (HTML — tool name wrapped in <b>)
        tr!(
            "hitl.exec_failed",
            "⚠️ <b>{0}</b> 执行失败: ",
            "⚠️ <b>{0}</b> execution failed: "
        );

        // ── Reminder / Scheduler proactive pushes (Channel B; i18n Task 3) ──
        // Fallback used when a `notify` intent has empty context_text.
        // {} = intent id.
        tr!(
            "reminder.fallback",
            "\u{23f0} 提醒 #{}",
            "\u{23f0} Reminder #{}"
        );
        // Scheduled-action timeout notice PREFIX (proactive). {} = retries. The
        // error text is concatenated by the caller AFTER this prefix and is NOT
        // routed through any placeholder substitution — an arbitrary error string
        // may itself contain `{}`, which a re-scanning formatter would corrupt.
        tr!(
            "reminder.scheduled_timed_out",
            "\u{26a0}\u{fe0f} 计划任务在 {} 次重试后超时：",
            "\u{26a0}\u{fe0f} Scheduled action timed out after {} retries: "
        );
        tr!(
            "reminder.scheduled_failed",
            "\u{26a0}\u{fe0f} 计划任务失败：{}",
            "\u{26a0}\u{fe0f} Scheduled action failed: {}"
        );
        // Native web-action hint appended to owner web reminder notices. Each
        // `{}` is replaced with the task id (tf replaces ALL `{}` occurrences).
        tr!(
            "reminder.web_actions",
            "网页操作：`done reminder {}`、`snooze reminder {}`、`tomorrow reminder {}` 或 `delete reminder {}`。",
            "Web actions: `done reminder {}`, `snooze reminder {}`, `tomorrow reminder {}`, or `delete reminder {}`."
        );

        // ── Reminder action callbacks (Telegram inline buttons; Channel B; i18n Task 3) ──
        tr!(
            "reminder.cb.invalid_id",
            "\u{274c} 无效提醒 ID",
            "\u{274c} Invalid reminder ID"
        );
        tr!(
            "reminder.cb.pick_snooze",
            "请选择推迟时间：",
            "Please pick a snooze duration:"
        );
        tr!(
            "reminder.cb.confirm_delete_prompt",
            "请确认删除这个提醒。",
            "Please confirm deleting this reminder."
        );
        tr!(
            "reminder.cb.not_found_or_cleared",
            "\u{274c} 提醒不存在或已清理",
            "\u{274c} Reminder does not exist or was already cleared"
        );
        tr!(
            "reminder.cb.not_found",
            "\u{274c} 找不到这个提醒",
            "\u{274c} Could not find this reminder"
        );
        tr!("reminder.cb.done", "\u{2705} 已完成", "\u{2705} Done");
        tr!(
            "reminder.cb.not_found_short",
            "\u{274c} 提醒未找到",
            "\u{274c} Reminder not found"
        );
        tr!(
            "reminder.cb.complete_failed",
            "\u{274c} 完成提醒失败",
            "\u{274c} Failed to complete reminder"
        );
        tr!(
            "reminder.cb.delete_failed",
            "\u{274c} 删除提醒失败",
            "\u{274c} Failed to delete reminder"
        );
        tr!(
            "reminder.cb.invalid_snooze",
            "\u{274c} 无效推迟选项",
            "\u{274c} Invalid snooze option"
        );
        // {} = human label of the snooze target time.
        tr!(
            "reminder.cb.snoozed_to",
            "\u{1f4a4} 已推迟到 {}",
            "\u{1f4a4} Snoozed to {}"
        );
        tr!(
            "reminder.cb.cannot_snooze",
            "\u{274c} 这个提醒不能推迟",
            "\u{274c} This reminder cannot be snoozed"
        );
        tr!(
            "reminder.cb.snooze_failed",
            "\u{274c} 推迟提醒失败",
            "\u{274c} Failed to snooze reminder"
        );
        tr!(
            "reminder.cb.unknown_action",
            "\u{274c} 未知提醒操作",
            "\u{274c} Unknown reminder action"
        );
        tr!(
            "reminder.cb.action_failed",
            "\u{274c} 提醒操作失败",
            "\u{274c} Reminder action failed"
        );
        // {} = snooze label.
        tr!(
            "reminder.cb.snoozed_archived",
            "\u{1f5d1} 已删除（可逆归档）",
            "\u{1f5d1} Deleted (reversible archive)"
        );

        // Reminder inline-keyboard button labels (Channel B).
        tr!("reminder.btn.done", "完成", "Done");
        tr!("reminder.btn.snooze", "推迟", "Snooze");
        tr!("reminder.btn.delete", "删除", "Delete");
        tr!("reminder.btn.details", "查看详情", "Details");
        tr!("reminder.btn.cancel", "取消", "Cancel");
        tr!("reminder.btn.confirm_delete", "确认删除", "Confirm delete");

        // Snooze duration labels (Channel B).
        tr!("reminder.snooze.10min", "10分钟", "10 min");
        tr!("reminder.snooze.1hour", "1小时", "1 hour");
        tr!("reminder.snooze.tonight", "今晚", "Tonight");
        tr!("reminder.snooze.tomorrow", "明天", "Tomorrow");
        tr!("reminder.snooze.next_monday", "下周一", "Next Monday");

        // Reminder details panel (Channel B). {0}=id {1}=status {2}=time {3}=title {4}=body
        tr!("reminder.details.title", "提醒详情", "Reminder details");
        tr!("reminder.details.status", "状态", "Status");
        tr!("reminder.details.time", "时间", "Time");
        tr!("reminder.details.title_label", "标题", "Title");
        tr!("reminder.details.unset", "未设置", "Not set");

        // ── WebApp attachment-only captions (Channel B — display_language) ──
        // A composer turn that carries an attachment but no typed text: the
        // orchestrator still needs a message, so the attachment supplies one
        // (see admin::webapp_gateway::vnext_command_effective_input). Mirrors
        // how the Telegram photo/document path narrates a caption-less send.
        tr!(
            "webapp.attachment.imageOnly",
            "[用户发送了一张图片]",
            "[The owner sent an image]"
        );
        tr!(
            "webapp.attachment.documentOnly",
            "[用户发送了一个文件]",
            "[The owner sent a document]"
        );

        // ── Panel-button action results (Channel B — owner display_language) ──
        // orchestrator_panels.rs handle_panel_submission: task_done / archive /
        // skip / restore / delete / dismiss / pin / lock / feedback button
        // results that render DIRECTLY to the owner. Previously hardcoded
        // bilingual or English literals; now resolve under the owner's
        // display_language. The *_failed keys are PREFIX-ONLY (an arbitrary
        // error string `e` is concatenated AFTER — never routed through tf/tfn,
        // which would re-scan a literal `{}` in the error and corrupt it; see
        // hitl_arbitrary_string_keys_are_prefix_only_no_tfn_rescan).
        tr!("panel.action.done", "✅ 已完成", "✅ Done");
        tr!(
            "panel.action.task_not_found_or_done",
            "任务未找到或已完成。",
            "Task not found or already completed."
        );
        // PREFIX — error `e` concatenated after
        tr!(
            "panel.action.complete_failed",
            "完成任务失败：",
            "Failed to complete task: "
        );
        tr!("panel.action.archived", "🗑 已归档", "🗑 Archived");
        tr!(
            "panel.action.task_not_found",
            "任务未找到。",
            "Task not found."
        );
        // PREFIX — error `e` concatenated after
        tr!(
            "panel.action.archive_failed",
            "归档任务失败：",
            "Failed to archive task: "
        );
        tr!(
            "panel.action.skipped",
            "⏭ 已跳过 — 已重新安排下次触发。",
            "⏭ Skipped — next occurrence rescheduled."
        );
        // PREFIX — error `e` concatenated after
        tr!("panel.action.skip_failed", "跳过失败：", "Failed to skip: ");
        tr!(
            "panel.action.done_not_recurring",
            "✅ 已完成（一次性任务）",
            "✅ Done (not recurring)"
        );
        tr!("panel.action.restored", "↩ 已恢复", "↩ Restored");
        // PREFIX — error `e` concatenated after
        tr!(
            "panel.action.restore_failed",
            "恢复失败：",
            "Restore failed: "
        );
        // {0} = count (safe numeric). Uses a positional token because tfp only
        // substitutes `{<digits>}` (bare `{}` is emitted literally).
        tr!(
            "panel.action.cleared_finished",
            "🧹 已清空 {0} 项已完成任务",
            "🧹 Cleared {0} finished task(s)"
        );
        // PREFIX — error `e` concatenated after
        tr!("panel.action.clear_failed", "清空失败：", "Clear failed: ");
        tr!(
            "panel.action.deleted",
            "🔴 已永久删除",
            "🔴 Permanently deleted"
        );
        tr!(
            "panel.action.delete_only_finished",
            "只能删除已完成/已归档的任务",
            "Can only delete done/archived tasks"
        );
        // PREFIX — error `e` concatenated after
        tr!(
            "panel.action.delete_failed",
            "删除失败：",
            "Delete failed: "
        );
        tr!("panel.action.pinned", "📌 已置顶", "📌 Pinned");
        tr!("panel.action.unpinned", "📌 已取消置顶", "📌 Unpinned");
        tr!("panel.action.locked", "🔒 已锁定", "🔒 Locked");
        tr!("panel.action.unlocked", "🔓 已解锁", "🔓 Unlocked");
        tr!(
            "panel.action.feedback_empty",
            "请先输入反馈内容再提交。",
            "Please enter feedback text before submitting."
        );
        tr!(
            "panel.action.panel_not_found",
            "面板未找到。",
            "Panel not found."
        );

        // ── Web console owner copy (zh first; no system jargon) ──
        // Banned in zh: 回执, 事项, 幕僚长, 主人, 边界, 拦截, 投影
        // Prefer: 记录 / 这件事 / 待办 / 你

        tr!(
            "console.home.brief.quiet",
            "这会儿没什么急着找你的。",
            "Nothing urgent is asking for you right now."
        );
        tr!(
            "console.home.brief.attention",
            "需要看一下：{0}。",
            "Needs attention: {0}."
        );
        tr!(
            "console.home.brief.decide",
            "要你拍板：{0}。",
            "Needs your decision: {0}."
        );
        tr!(
            "console.home.brief.coming_up",
            "接下来：{0}。",
            "Coming up: {0}."
        );
        tr!(
            "console.home.brief.ready",
            "可以看一下：{0}。",
            "Ready to review: {0}."
        );
        tr!(
            "console.home.brief.follow_up",
            "跟进中：{0}。",
            "Following up: {0}."
        );
        tr!(
            "console.home.brief.backlog",
            "先放着：{0}。",
            "Quietly saved for later: {0}."
        );
        tr!(
            "console.home.brief.start",
            "从这件事开始：{0}。",
            "Start with {0}."
        );
        tr!("console.home.fallback_item", "这件事", "this item");
        tr!("console.item.work_fallback", "待办", "Work item");
        tr!("console.work.lane.now", "现在", "Now");
        tr!("console.work.lane.decide", "要你决定", "Decide");
        tr!("console.work.lane.follow_up", "跟进", "Follow Up");
        tr!("console.work.lane.backlog", "以后再说", "Backlog");
        tr!("console.work.lane.done", "已完成", "Done");
        tr!(
            "console.work.receipts.grouped",
            "最近记录：{0} 条已完成（{1}…）",
            "{0} completed today ({1}…)"
        );
        tr!(
            "console.work.receipts.grouped.reason",
            "今天做完的记录收成一条，需要时再打开看。",
            "Completed records for the day are grouped here; open to inspect any one."
        );
        tr!(
            "console.work.receipts.grouped.summary",
            "已完成 {0} 条。",
            "{0} completed."
        );
        tr!(
            "console.work.stale.goal",
            "30 天没有进展，先收起来",
            "No progress for 30 days — parked"
        );
        tr!(
            "console.work.stale.tracking",
            "14 天没有动静，先收起来",
            "No update for 14 days — parked"
        );
        tr!(
            "console.work.stale.fired",
            "提醒已经响过，先收起来",
            "Already fired — parked"
        );
        tr!("console.work.duplicate.meta", "×{0}", "×{0}");
        tr!("console.item.receipt.record", "这条记录", "Receipt record");
        tr!(
            "console.collab.placeholder",
            "随便问我这件事。",
            "Ask Aegis anything about this item."
        );

        tr!("console.status.health.label", "系统状况", "System Health");
        tr!(
            "console.status.health.ready",
            "该连的都连上了，没连的先放着。",
            "Connected sources are ready or intentionally quiet."
        );
        tr!(
            "console.status.health.one_attention",
            "有一路连接需要你看一下。",
            "One connection needs attention."
        );
        tr!(
            "console.status.health.more_limited",
            "{0}；另外 {1} 路也受限。",
            "{0}; {1} more connection(s) limited."
        );
        tr!(
            "console.status.health.n_limited",
            "有 {0} 路连接受限。",
            "{0} connections are limited."
        );
        tr!(
            "console.status.health.inspect",
            "先看看出问题的那一路，再重试相关工作。",
            "Inspect the affected source before retrying dependent work."
        );
        tr!(
            "console.status.background.label",
            "后台整理",
            "Background Review"
        );
        tr!(
            "console.status.background.open",
            "我在把进行中的事理顺，不会每查一次都打扰你。",
            "Aegis is keeping current open loops organized without narrating every check."
        );
        tr!(
            "console.status.background.quiet",
            "现在没有进行中的事需要后台汇报。",
            "No current open loop needs a background status update."
        );
        tr!(
            "console.status.background.next",
            "想看正在忙的，打开工作页就行。",
            "Open Work when you want to review the live queue."
        );
        tr!(
            "console.status.recommendations.label",
            "建议",
            "Recommendations"
        );
        tr!(
            "console.status.recommendations.need_choice",
            "有 {0} 件事要你先选，我才能继续。",
            "{0} item(s) need your choice before Aegis goes further."
        );
        tr!(
            "console.status.recommendations.none",
            "这会儿没有要你拍板的。",
            "No owner decision is waiting right now."
        );
        tr!(
            "console.status.recommendations.next",
            "打开工作页，先处理标出来的那件。",
            "Open Work and decide the highlighted item before Aegis continues."
        );
        tr!("console.status.learning.label", "记忆", "Learning");
        tr!(
            "console.status.learning.message",
            "我会参考你说过的偏好，改起来也随时能看。",
            "Aegis may use remembered preferences as context, but corrections stay reviewable."
        );
        tr!(
            "console.status.standing.label",
            "长期交代",
            "Standing Instructions"
        );
        tr!(
            "console.status.standing.message",
            "反复出现的责任，应能看出是哪条交代带出来的。",
            "Recurring responsibilities should cite the instruction that surfaced them."
        );
        tr!("console.status.followups.label", "跟进", "Follow-ups");
        tr!(
            "console.status.followups.some",
            "有 {0} 件在等或在跟，到点会再出现。",
            "{0} waiting or tracking loop(s) will resurface when useful."
        );
        tr!(
            "console.status.followups.none",
            "这会儿没有在等的事。",
            "No waiting loop needs attention right now."
        );
        tr!(
            "console.status.followups.next",
            "打开工作页，看看我在跟什么。",
            "Open Work to inspect what Aegis is tracking."
        );
        tr!("console.status.receipts.label", "记录", "Receipts");
        tr!(
            "console.status.receipts.message",
            "对外做完的事，我仍要先有记录才算数。",
            "Aegis still needs receipts before claiming external work is done."
        );
        tr!("console.status.session.label", "你的登录", "Owner session");
        tr!(
            "console.status.session.message",
            "已用这台服务器上的登录状态签到。",
            "Signed in with the server-owned owner session."
        );
        tr!("console.status.ready", "就绪", "Ready");
        tr!("console.status.cost.label", "省钱控制", "Cost controls");
        tr!(
            "console.status.cost.paused",
            "省钱模式开着：我自己发起的后台活先停了。你点的对话、工作和检查照常跑。",
            "Cost-saving mode is on: self-started background work is paused. Chat, Work actions, and checks requested by you still run."
        );
        tr!(
            "console.status.cost.summary",
            "后台花费控制：{0}。",
            "Background cost controls: {0}."
        );
        tr!(
            "console.status.cost.next",
            "开发期控完花费后，想恢复后台自动跑，去设置里打开。",
            "Use Settings when you want to reopen background autonomy after development cost control."
        );
        tr!(
            "console.status.cost.review_on",
            "持续后台整理开着",
            "continuous background review is enabled"
        );
        tr!(
            "console.status.cost.review_off",
            "持续后台整理已暂停",
            "continuous background review is paused"
        );
        tr!(
            "console.status.cost.budget_off",
            "后台推理额度已暂停",
            "background reasoning budget is paused"
        );
        tr!(
            "console.status.cost.budget_on",
            "后台推理额度可以跑",
            "background reasoning budget can run"
        );
        tr!(
            "console.status.cost.skills_none",
            "还没登记定时技能检查",
            "no scheduled skill checks are registered"
        );
        tr!(
            "console.status.cost.skills_paused",
            "{0} 项定时技能检查已暂停",
            "{0} scheduled skill checks are paused"
        );
        tr!(
            "console.status.cost.skills_partial",
            "{0} / {1} 项定时技能检查可以跑",
            "{0} of {1} scheduled skill checks can run"
        );
        tr!(
            "console.status.cost.health_off",
            "系统健康检查已暂停",
            "system health checks are paused"
        );
        tr!(
            "console.status.cost.health_every",
            "系统健康检查每 {0} 分钟一次",
            "system health checks every {0} minutes"
        );

        tr!("console.source.action_ledger", "记录", "Receipts");
        tr!("console.source.alerts", "系统提醒", "System alerts");
        tr!("console.source.calendar", "日历", "Calendar");
        tr!("console.source.goals", "目标", "Goals");
        tr!("console.source.hitl", "待你确认", "Approvals");
        tr!("console.source.knowledge", "在等的事", "Waiting items");
        tr!(
            "console.source.mandates",
            "长期交代",
            "Standing instructions"
        );
        tr!("console.source.memory", "记住的东西", "Learning");
        tr!("console.source.tasks", "待办和提醒", "Tasks and reminders");
        tr!(
            "console.source.status.auth",
            "Google 登录需要刷新",
            "Google sign-in needs refresh"
        );
        tr!(
            "console.source.status.permission",
            "还缺权限",
            "Permission needed"
        );
        tr!(
            "console.source.status.unavailable",
            "这会儿连不上",
            "Unavailable right now"
        );
        tr!("console.source.status.error", "出错了", "Error");
        tr!("console.source.status.failed", "失败了", "Failed");
        tr!(
            "console.source.status.delegated",
            "Codex 访问待验证",
            "Codex access not yet verified"
        );
        tr!(
            "console.source.reason.metadata",
            "只用了安全的基本信息；没打开里面的内容。",
            "Using safe metadata only; private source content was not opened."
        );
        tr!(
            "console.source.reason.not_connected",
            "这路还没接上，先安静放着。",
            "Intentionally quiet until this source is connected."
        );
        tr!(
            "console.source.reason.projector",
            "这个信号还没接到你能看到的页面。",
            "This signal is not wired into the owner surface yet."
        );
        tr!(
            "console.source.next.auth",
            "先在设置里刷新 Google 连接，再指望新的私人更新。",
            "Refresh the Google connection from Settings before expecting new private updates."
        );
        tr!(
            "console.source.next.permission",
            "先补上缺的权限，我才能把这路用全。",
            "Grant the missing permission before Aegis can use this source fully."
        );
        tr!(
            "console.source.next.unavailable",
            "过会儿再试；一直这样的话，去设置里看连接。",
            "Try again later; if it persists, check the service connection in Settings."
        );
        tr!("console.source.attention", "{0}：{1}。", "{0}: {1}.");

        tr!(
            "console.event.restart.title",
            "系统刚重启",
            "System restarted"
        );
        tr!(
            "console.event.restart.summary",
            "我重启完了，已经回来。",
            "Aegis restarted and returned to service."
        );
        tr!(
            "console.event.integrity.title",
            "完整性报告",
            "Integrity report"
        );
        tr!(
            "console.event.integrity.warn",
            "完整性报告里有个趋势，值得在状态页看一眼。",
            "A system integrity report found a trend worth reviewing in Status."
        );
        tr!(
            "console.event.integrity.ok",
            "完整性报告跑完了，不用你动手。",
            "A system integrity report completed without needing owner action."
        );
        tr!(
            "console.event.brief.title",
            "今日摘要不完整",
            "Incomplete daily brief"
        );
        tr!(
            "console.event.brief.summary",
            "今日摘要没去问日历，所以可能不全。",
            "A daily brief ran without querying Calendar, so it may be incomplete."
        );

        tr!(
            "console.settings.push.label",
            "网页通知",
            "WebApp notifications"
        );
        tr!(
            "console.settings.push.ready",
            "这台服务器可以登记并发送已安装网页的通知。",
            "This server can register and send installed WebApp notifications."
        );
        tr!(
            "console.settings.push.partial",
            "可以登记设备，但发送还缺配对的私钥。",
            "This server can register devices, but delivery needs the matching VAPID private key."
        );
        tr!(
            "console.settings.push.missing",
            "网页推送还没配好，已安装的网页暂时收不到手机通知。",
            "Web Push needs VAPID configuration before the installed WebApp can receive phone notifications."
        );
        tr!(
            "console.settings.push.next.ready",
            "浏览器弹出时，在这台设备上打开通知。",
            "Enable notifications from this device when the browser offers it."
        );
        tr!(
            "console.settings.push.next.partial",
            "配好发送私钥，再发一条测试通知。",
            "Configure the private send key, then send a test notification."
        );
        tr!(
            "console.settings.push.next.missing",
            "网页推送配好前，紧急提醒先走 Telegram。",
            "Keep Telegram as the immediate notification fallback until Web Push is configured."
        );
        tr!("console.settings.telegram.label", "Telegram", "Telegram");
        tr!(
            "console.settings.telegram.message",
            "网页推送成熟前，提醒和紧急告警仍可走 Telegram。",
            "Telegram remains the fallback phone rail for reminders and urgent alerts while Web Push matures."
        );
        tr!(
            "console.settings.calendar.delegated",
            "日历读取需要 Codex 连接器；执行前会验证访问权限。",
            "Calendar reads require a Codex connector; access is verified before execution."
        );
        tr!(
            "console.push.unconfigured",
            "服务器还没配网页推送。紧急通知先走 Telegram。",
            "Web Push is not configured on the server yet. Telegram remains the phone notification fallback."
        );
        tr!(
            "console.push.no_send_key",
            "这台设备可以订阅，但服务器还缺配对私钥，暂时发不出去。",
            "This device can subscribe, but the server still needs the matching VAPID private key before it can send notifications."
        );
        tr!(
            "console.push.subscribed",
            "这个账号已经登记了至少一台网页通知设备。",
            "This owner account has at least one WebApp notification device registered."
        );
        tr!(
            "console.push.ready",
            "服务器已准备好网页通知。从已安装的浏览器应用打开即可。",
            "This server is ready for WebApp notifications. Enable them from an installed browser app."
        );
        tr!(
            "console.push.next.unconfigured",
            "服务器还没配好网页推送密钥；配好前，提醒照常走 Telegram。",
            "The server has no Web Push keys yet; Telegram keeps delivering reminders until it does."
        );
        tr!(
            "console.push.next.no_send_key",
            "服务器还缺发送密钥；补上后就能推到这台设备。",
            "The server still lacks its send key; once added, notifications reach this device."
        );
        tr!(
            "console.push.next.subscribed",
            "先从这台设备发一条测试通知，再把提醒从 Telegram 挪过来。",
            "Send a test notification from this device before moving reminders from Telegram fallback."
        );
        tr!(
            "console.push.next.ready",
            "在已安装的网页里点开启，并允许浏览器通知。",
            "Tap Enable from the installed WebApp and approve the browser notification prompt."
        );
        tr!(
            "console.push.none_registered",
            "这个账号还没登记网页通知设备。",
            "No WebApp notification device is registered for this owner yet."
        );
        tr!(
            "console.push.notice.fallback",
            "打开 Aegis 看一下更新。",
            "Open Aegis to review the update."
        );

        tr!(
            "console.decide.blocked.title.web_search",
            "网上搜索要你选一下",
            "Web search needs your choice"
        );
        tr!(
            "console.decide.blocked.title.generic",
            "自动流程要你选一下",
            "Automation needs your choice"
        );
        tr!(
            "console.decide.blocked.cap.web_search",
            "为这次定时请求使用网上搜索。",
            "Use the web search tool for this scheduled request."
        );
        tr!(
            "console.decide.blocked.cap.generic",
            "让这次自动流程越过受保护的一步继续。",
            "Continue past the protected boundary for this automation."
        );
        tr!(
            "console.decide.choice.run_once",
            "这次准跑",
            "Approve this run"
        );
        tr!(
            "console.decide.choice.run_once.consequence",
            "我会把这次暂停的跑完一遍，并记下你准了。",
            "Aegis will continue this paused run once and record the approval path."
        );
        tr!(
            "console.decide.choice.skip_once",
            "这次跳过",
            "Skip this run"
        );
        tr!(
            "console.decide.choice.skip_once.consequence",
            "这次跳过，定时规则本身不动。",
            "Aegis will skip this paused run and leave the recurring rule unchanged."
        );
        tr!(
            "console.decide.choice.approve_unavailable.consequence",
            "这张卡片上没有还有效的确认请求，所以没法从这里继续。",
            "Aegis cannot continue from this card because the live approval request is missing or expired."
        );
        tr!(
            "console.decide.choice.skip_unavailable.consequence",
            "原来的确认请求已经不在了，没法从这里跳过。",
            "Aegis cannot skip the original approval request because that request is no longer pending."
        );
        tr!(
            "console.decide.choice.disabled_no_approval",
            "这次暂停没有挂上还有效的确认请求。",
            "No live approval request is attached to this pause."
        );
        tr!("console.decide.choice.revise", "改规则", "Revise rule");
        tr!(
            "console.decide.choice.revise.consequence",
            "先跟我把规则收窄或说清楚，再让任何一次跑下去。",
            "Discuss a narrower or clearer rule with Aegis before any run continues."
        );
        tr!(
            "console.decide.choice.revise.prompt",
            "帮我改这条暂停的自动规则。先说明它想做什么，再给更安全、更窄的选项。不要对外执行任何动作。",
            "Help me revise this paused automation rule. First explain what it is trying to do, then propose safer narrower options. Do not execute any external action."
        );
        tr!(
            "console.decide.choice.view_details",
            "看安全详情",
            "View safe details"
        );
        tr!(
            "console.decide.choice.view_details.consequence",
            "打开这件事的上下文和历史，不会跑被挡住的那一步。",
            "Open the task context and history without running the blocked step."
        );
        tr!(
            "console.decide.choice.review_tomorrow",
            "明天再看",
            "Review tomorrow"
        );
        tr!(
            "console.decide.choice.review_tomorrow.consequence",
            "先停着，明天再拿给你看。",
            "Aegis will keep this paused and surface it again tomorrow."
        );
        tr!(
            "console.decide.choice.close",
            "不跑，直接关掉",
            "Close without running"
        );
        tr!(
            "console.decide.choice.close.consequence",
            "我会把这件暂停的待办标成完成。不会读邮件、不会跑被挡住的那步、也不会继续这件事。",
            "Aegis will mark this paused task done. It will not read mail, run the blocked step, or continue this task."
        );
        tr!(
            "console.decide.safe.no_external",
            "还没有对外做过任何事。",
            "No external action has run."
        );
        tr!(
            "console.decide.safe.visible_metadata",
            "能看到的基本信息可以先看，不必打开私人内容。",
            "Visible metadata can be reviewed without opening private content."
        );
        tr!(
            "console.decide.safe.no_live_approval",
            "这次暂停没有挂上还有效的确认请求，所以准跑/跳过先不可用，等我重新生成真正的确认请求。",
            "No live approval request is attached to this pause, so approve/skip is unavailable until Aegis regenerates a real approval request."
        );
        tr!(
            "console.decide.scope.blocked_run",
            "准跑或跳过只针对这次暂停。关掉等于把这件暂停的待办标成完成。改规则只有你明确选择时才会发生。",
            "Approve or skip applies to this paused run. Closing marks this paused task done. Rule changes only happen if you explicitly choose a rule change."
        );
        tr!(
            "console.decide.approval.title_fallback",
            "要你决定",
            "Decision needed"
        );
        tr!("console.decide.approval.approve", "同意", "Approve");
        tr!(
            "console.decide.approval.approve.consequence",
            "我会沿现有确认路径送出这次同意。",
            "Aegis will send this approval through the existing approval path."
        );
        tr!("console.decide.approval.reject", "拒绝", "Reject");
        tr!(
            "console.decide.approval.reject.consequence",
            "我会沿现有确认路径拒绝这次请求。",
            "Aegis will reject this request through the existing approval path."
        );
        tr!(
            "console.decide.approval.scope",
            "只针对这次确认请求。",
            "This approval request only."
        );
        tr!(
            "console.decide.approval.safe.no_outside",
            "你选之前，不会对外做任何事。",
            "No outside action runs until you choose."
        );

        tr!(
            "console.prompt.what_should",
            "这件事我该怎么处理？",
            "What should I do with this?"
        );
        tr!(
            "console.prompt.remind_tomorrow",
            "如果还没处理，明天再提醒我。",
            "Remind me tomorrow if this is not handled."
        );
        tr!(
            "console.prompt.break_steps",
            "拆成接下来能动手的几步。",
            "Break this into the next concrete steps."
        );
        tr!(
            "console.prompt.track",
            "跟到这件事解决为止。",
            "Track this until it is resolved."
        );
        tr!(
            "console.prompt.backlog",
            "除非现在就要我，先放到以后再说。",
            "Move this to backlog unless it needs me now."
        );

        tr!(
            "console.blocked.title.web_search",
            "网上搜索已暂停",
            "Web search paused"
        );
        tr!(
            "console.blocked.title.generic",
            "自动流程已暂停",
            "Automation paused"
        );
        tr!(
            "console.blocked.summary.web_search",
            "定时搜索已暂停：{0}",
            "Scheduled search paused: {0}"
        );
        tr!(
            "console.blocked.summary.generic",
            "自动流程在继续前暂停了：{0}",
            "Automation paused before continuing: {0}"
        );
        tr!(
            "console.blocked.reason.web_search",
            "这可能调用外部搜索，所以我先停住了。",
            "This may call an external search tool, so Aegis paused before continuing."
        );
        tr!(
            "console.blocked.reason.generic",
            "敏感的一步要你看过，我才继续。",
            "A sensitive step needs owner review before Aegis continues."
        );
        tr!(
            "console.blocked.next.web_search",
            "准这次搜索、跳过，或让我改规则。",
            "Approve this search run, skip it, or ask Aegis to revise the rule."
        );
        tr!(
            "console.blocked.next.generic",
            "看看这次暂停，让我改自动流程，或关掉它。",
            "Review this pause, ask Aegis to revise the automation, or close it."
        );
        tr!(
            "console.blocked.by.web_search",
            "对外搜索这一步要你点头。",
            "Running the external search step needs owner permission."
        );
        tr!(
            "console.blocked.by.generic",
            "敏感一步之前，我先把自动流程停住了。",
            "Aegis paused this automation before a sensitive step."
        );
        tr!(
            "console.blocked.what.web_search",
            "跑这次定时的网上搜索。",
            "Run the scheduled web search request."
        );
        tr!(
            "console.blocked.what.generic",
            "继续一件已经走到受保护一步的自动流程。",
            "Continue an automation that reached a protected boundary."
        );
        tr!(
            "console.blocked.what.web_search.generic_summary",
            "我正准备为一次定时请求做网上搜索。",
            "Aegis was about to run a web search for a scheduled request."
        );
        tr!(
            "console.blocked.task.next",
            "准这次跑、跳过，或先让我改规则。",
            "Approve this run, skip it, or ask Aegis to revise the rule first."
        );
        tr!("console.blocked.this_request", "这次请求", "this request");

        tr!(
            "console.calendar.summary",
            "日历{0}；结束 {1}；{2} 位参与者；私人细节先关上",
            "Calendar {0}; ends {1}; {2} attendee(s); private details stay closed"
        );
        tr!(
            "console.calendar.reason",
            "日历基本信息有变，可能影响安排。",
            "Calendar basics changed or may affect planning."
        );
        tr!(
            "console.calendar.action.review",
            "看日历基本信息",
            "Review calendar basics"
        );
        tr!(
            "console.calendar.next",
            "先看时间安排的基本信息，再让我推理私人细节。",
            "Review the schedule basics before asking Aegis to reason over private details."
        );
        tr!(
            "console.calendar.blocked",
            "日历描述、地点和会议细节不在这一行里。",
            "Private calendar description, location, and meeting details are not in this queue row."
        );
        tr!(
            "console.calendar.receipt",
            "还没有改过日历。以后若要写入，必须留下服务商记录。",
            "No calendar action has been executed. Any future calendar write must create a provider receipt."
        );
        tr!(
            "console.calendar.route",
            "日历基本信息可用；私人细节先关上",
            "Calendar basics are available; private details stay closed"
        );
        tr!("console.calendar.end.unknown", "未知", "unknown");
        tr!("console.calendar.action.changed", "有更新", "changed");

        tr!(
            "console.mandate.title",
            "规则对上了：{0}",
            "Rule matched: {0}"
        );
        tr!(
            "console.mandate.summary",
            "{0}对上了这条{1}。私人细节要你打开才会看。",
            "{0} matched this {1} item. Private details stay closed until you open them."
        );
        tr!(
            "console.mandate.reason",
            "一条生效中的规则对上了这件事的基本来源信息。",
            "An active rule matched this item's basic source information."
        );
        tr!(
            "console.mandate.next",
            "只有你想让我细看时，再打开具体来源。",
            "Open the exact source only if you want Aegis to inspect it."
        );
        tr!(
            "console.mandate.blocked",
            "私人来源细节要打开具体条目才会解锁。",
            "Private source details remain locked until an exact item is opened."
        );
        tr!(
            "console.mandate.receipt.planned",
            "已挂上脱敏后的提醒计划；还没对来源做过任何动作。",
            "A redacted notification plan is linked; no source action has been executed."
        );
        tr!(
            "console.mandate.receipt.none",
            "还没有对外做过任何事。打开来源只给你阅读上下文。",
            "No external action has been executed. Opening the source only grants read context."
        );
        tr!(
            "console.mandate.route",
            "生效规则对上了；提醒{0}。",
            "Active rule matched; reminder {0}."
        );
        tr!(
            "console.mandate.notify.title",
            "规则对上了：{0}",
            "Rule matched: {0}"
        );
        tr!(
            "console.mandate.notify.summary",
            "有一条{0}对上了生效规则。打开网页看具体是哪件。",
            "A {0} item matched an active rule. Open WebApp for the exact item."
        );
        tr!("console.event_source.calendar", "日历", "calendar");
        tr!("console.event_source.drive", "云盘", "Drive");
        tr!("console.event_source.chronos", "定时任务", "scheduler");
        tr!("console.event_source.telegram", "Telegram", "Telegram");
        tr!("console.event_source.browser", "浏览器", "browser");
        tr!("console.event_source.unknown", "助理", "assistant");
        tr!("console.mandate.scope.email", "邮件抬头", "Email headers");
        tr!("console.mandate.scope.calendar", "日历", "Calendar");
        tr!("console.mandate.scope.reminders", "提醒", "Reminders");
        tr!("console.mandate.scope.alerts", "系统提醒", "System alerts");
        tr!("console.mandate.scope.costs", "接口花费", "API costs");
        tr!(
            "console.mandate.scope.approvals",
            "确认和记录",
            "Approvals and receipts"
        );
        tr!("console.mandate.scope.memory", "记忆复查", "Memory review");
        tr!("console.mandate.scope.generic", "规则范围", "Rule scope");
        tr!("console.mandate.notify_status.ready", "已准备", "Ready");
        tr!(
            "console.mandate.notify_status.shown",
            "已经给你看过",
            "Already shown"
        );
        tr!(
            "console.mandate.notify_status.cap",
            "今天次数用完了",
            "Daily limit reached"
        );
        tr!("console.mandate.notify_status.sent", "已发出", "Sent");
        tr!(
            "console.mandate.notify_status.cancelled",
            "已取消",
            "Cancelled"
        );
        tr!("console.mandate.notify_status.waiting", "还在等", "Waiting");
        tr!(
            "console.mandate.reminder.not_prepared",
            "还没准备",
            "not prepared"
        );

        tr!(
            "console.delegation.reason",
            "按你的要求，我把这件事交给了 Hercules。",
            "Delegated to Hercules by Aegis at your request."
        );
        tr!(
            "console.delegation.next.review",
            "在 Hercules 里看 PR；在那边批准，或让我去带这件事。",
            "Review the PR in Hercules; approve there or ask Aegis to steer the task."
        );
        tr!(
            "console.delegation.next.failed",
            "问我近况；操作员可以在 Hercules 里重试。",
            "Ask Aegis for status; an operator can retry the task in Hercules."
        );
        tr!(
            "console.delegation.next.done",
            "Hercules 已核实。合并任务分支是 Hercules 那边另一步（合并队列）。",
            "Verified by Hercules. Merging the task branch is a separate Hercules step (merge queue)."
        );
        tr!(
            "console.delegation.next.cancelled",
            "没什么要做的。",
            "Nothing to do."
        );
        tr!(
            "console.delegation.next.missing",
            "Hercules 已经不认得这件事；如果还重要，再委派一次。",
            "Hercules no longer knows this task; delegate again if it still matters."
        );
        tr!(
            "console.delegation.next.wait",
            "等一下；我每 30 秒刷新一次 Hercules 状态。",
            "Wait; Aegis refreshes Hercules state every 30 s."
        );
        tr!(
            "console.delegation.next.hold_ready",
            "Hercules 还按着：仓库还没标就绪。就绪检查过了，再在 Hercules 里标就绪。",
            "Hercules is holding this task: the repo is not marked ready. Mark it ready in Hercules once its readiness check passes."
        );
        tr!(
            "console.delegation.next.hold_approval",
            "Hercules 在等它那边的一次确认。",
            "Hercules is holding this task for an approval on its side."
        );
        tr!(
            "console.delegation.next.hold_other",
            "Hercules 按着这件事（看按住原因）；Aegis 这边不用动手。",
            "Hercules is holding this task (see the hold reason); nothing to do in Aegis."
        );
        tr!(
            "console.delegation.receipt",
            "Hercules 的核实（已核实 SHA + PR）就是记录；没有它，我绝不声称做完。",
            "Hercules verification (verified SHA + PR) is the receipt; Aegis never claims completion before it."
        );
        tr!(
            "console.delegation.summary",
            "Hercules · {0} · {1}",
            "Hercules · {0} · {1}"
        );
        tr!("console.delegation.state.review", "待你看", "Needs you");
        tr!("console.delegation.state.failed", "失败了", "Failed");
        tr!("console.delegation.state.done", "已完成", "Done");
        tr!("console.delegation.state.cancelled", "已取消", "Cancelled");
        tr!("console.delegation.state.missing", "找不到了", "Missing");
        tr!("console.delegation.state.active", "进行中", "In progress");
        tr!("console.delegation.verified", "已核实 {0}", "verified {0}");
        tr!("console.delegation.pr", "PR：{0}", "PR: {0}");
        tr!(
            "console.delegation.route.review",
            "有一个 PR 等你看。",
            "A pull request is waiting for you."
        );
        tr!(
            "console.delegation.route.failed",
            "这件事没有做成。",
            "This task failed."
        );
        tr!(
            "console.delegation.route.done",
            "Hercules 已核实。",
            "Hercules verified this."
        );
        tr!(
            "console.delegation.route.cancelled",
            "没什么要做的。",
            "Nothing to do."
        );
        tr!(
            "console.delegation.route.missing",
            "Hercules 已经不认得这件事。",
            "Hercules no longer has this task."
        );
        tr!(
            "console.delegation.route.wait",
            "还在进行。",
            "Still in progress."
        );
        tr!(
            "console.delegation.route.hold",
            "Hercules 先按着这件事。",
            "Hercules is holding this."
        );
        tr!(
            "console.delegation.source_unavailable",
            "委派记录暂时读不出来。",
            "Delegation records are unreadable right now."
        );
        tr!(
            "console.research.reason",
            "一次调研，不是每次查找一行。",
            "One research pass, not a row per lookup."
        );
        tr!(
            "console.research.next",
            "看调研结果。",
            "Read the research result."
        );
        tr!(
            "console.research.receipt",
            "一条调研记录。",
            "One research record."
        );
        tr!(
            "console.research.route",
            "这是一次调研。",
            "This is one research pass."
        );
        tr!(
            "console.research.source_unavailable",
            "调研记录暂时读不出来。",
            "Research records are unreadable right now."
        );
        tr!(
            "console.event.restart.skills",
            "活跃技能：{0}",
            "Active skills: {0}"
        );
        tr!(
            "console.event.restart.pending",
            "待你确认：{0}",
            "Waiting for you: {0}"
        );

        tr!(
            "console.approval.title.tool",
            "允许接上这个工具？",
            "Approve tool connection?"
        );
        tr!(
            "console.approval.title.generic",
            "需要你确认",
            "Approval required"
        );
        tr!(
            "console.approval.summary.mcp",
            "我想接上 MCP 服务器 `{0}`，好在这次会话里用 `{1}`。{2}",
            "Aegis wants to connect MCP server `{0}` so it can use `{1}` in this session.{2}"
        );
        tr!(
            "console.approval.summary.mcp.command",
            " 命令：{0}。",
            " Command: {0}."
        );
        tr!(
            "console.approval.reason.tool",
            "接上新的外部工具服务器前，要你明确点头。",
            "Aegis needs your explicit approval before using a new external tool server."
        );
        tr!(
            "console.approval.reason.generic",
            "要改应用外面的东西前，要你点头。",
            "Aegis needs your approval before changing anything outside the app."
        );
        tr!(
            "console.approval.next.tool",
            "只有你信任这个工具服务器用于当前会话，才同意；否则拒绝。",
            "Approve only if you trust this tool server for the current session; otherwise reject."
        );
        tr!(
            "console.approval.next.high_risk",
            "这是高风险确认，同意前请核验。",
            "Review the high-risk approval and verify if you choose to approve."
        );
        tr!(
            "console.approval.next.generic",
            "看过这次确认，再同意或拒绝。",
            "Review the approval, then approve or reject."
        );
        tr!(
            "console.approval.blocked.waiting",
            "在等你同意或拒绝。",
            "Waiting for your approve or reject decision."
        );
        tr!(
            "console.approval.blocked.mfa",
            "执行前还要一次刚做过的身份核验。",
            "Approval requires a recent MFA check before execution."
        );
        tr!(
            "console.approval.blocked.owner",
            "在等你决定。",
            "Waiting for the owner decision."
        );
        tr!(
            "console.approval.action.review",
            "看这次确认",
            "Review approval"
        );
        tr!("console.approval.action.approve", "同意", "Approve");
        tr!("console.approval.action.reject", "拒绝", "Reject");
        tr!(
            "console.approval.receipt",
            "若同意，执行必须先留下动作记录，我才能声称做完。",
            "If approved, execution must create an Action Ledger receipt before Aegis can claim completion."
        );
        tr!(
            "console.approval.summary.generic",
            "{0}。{1}。我在应用外动手前，请你先看一眼。",
            "{0}. {1}. Review it before Aegis does anything outside the app."
        );
        tr!(
            "console.approval.kind.settings",
            "改设置请求",
            "Settings change request"
        );
        tr!(
            "console.approval.kind.tool_conn",
            "接上工具请求",
            "Tool connection request"
        );
        tr!("console.approval.kind.tool", "工具请求", "Tool request");
        tr!(
            "console.approval.kind.waiting",
            "在等你确认的请求",
            "Request waiting for approval"
        );
        tr!(
            "console.approval.server.unknown",
            "未知服务器",
            "unknown server"
        );
        tr!("console.approval.tool.unknown", "未知工具", "unknown tool");

        tr!("console.task.action.open", "打开详情", "Open details");
        tr!(
            "console.task.action.close_without_running",
            "不跑，直接关掉",
            "Close without running"
        );
        tr!(
            "console.task.action.review_tomorrow",
            "明天再看",
            "Review tomorrow"
        );
        tr!("console.task.action.complete", "完成", "Complete");
        tr!("console.task.action.snooze", "推迟", "Snooze");
        tr!(
            "console.tracking.action.skip",
            "只结束这一次",
            "End only this time"
        );
        tr!(
            "console.tracking.action.cancel",
            "关闭整条跟踪",
            "Close this tracking"
        );
        tr!(
            "console.tracking.action.end_once",
            "结束这一次",
            "End this card"
        );
        tr!(
            "console.tracking.cancelled",
            "整条跟踪已关闭。已有记录会留下。",
            "This tracking is closed. History stays."
        );
        tr!(
            "console.tracking.skipped",
            "这一次已跳过。跟踪还在，下次到点还会出现。",
            "Skipped this time. Tracking stays and returns at the next fire."
        );
        tr!(
            "console.lineage.intent.recurring",
            "循环跟踪",
            "Recurring tracking"
        );
        tr!("console.lineage.intent.watch", "持续守望", "Standing watch");
        tr!(
            "console.lineage.intent.oneshot",
            "一次性安排",
            "One-time intent"
        );
        tr!(
            "console.lineage.intent.parent_goal",
            "上层目标",
            "Parent goal"
        );
        tr!("console.lineage.occurrence.card", "这张卡片", "This card");
        tr!(
            "console.lineage.occurrence.waiting",
            "这条等待",
            "This waiting note"
        );
        tr!("console.lineage.occurrence.goal", "这个目标", "This goal");
        tr!(
            "console.lineage.origin.context",
            "当时的说明",
            "Original instruction"
        );
        tr!("console.lineage.origin.message", "对话消息", "Chat message");
        tr!("console.lineage.origin.record", "记录", "Record");
        tr!("console.lineage.taskRef", "任务 {}", "Task {}");
        tr!("console.lineage.everyDay", "每天", "Every day");
        tr!("console.lineage.everyDays", "每 {} 天", "Every {} days");
        tr!("console.lineage.everyHour", "每小时", "Every hour");
        tr!("console.lineage.everyHours", "每 {} 小时", "Every {} hours");
        tr!("console.lineage.everyMinute", "每分钟", "Every minute");
        tr!(
            "console.lineage.everyMinutes",
            "每 {} 分钟",
            "Every {} minutes"
        );
        tr!("console.lineage.everySecond", "每秒", "Every second");
        tr!(
            "console.lineage.everySeconds",
            "每 {} 秒",
            "Every {} seconds"
        );
        tr!("console.lineage.who.you", "你", "You");
        tr!("console.lineage.who.aegis", "安琪", "Aegis");
        tr!(
            "console.task.next.closed",
            "只有需要记录或历史时，再打开详情。",
            "Open details only if you need receipts or history."
        );
        tr!(
            "console.task.next.open",
            "打开详情看私人待办，或你已经认得的话选一个安全动作。",
            "Open details to reveal the private task, or choose a safe action if you already recognize it."
        );
        tr!(
            "console.task.receipt.closed",
            "已关掉的待办，若当时留过记录，详情里还能看到。",
            "Closed task receipts are available from task details when a receipt was recorded."
        );
        tr!(
            "console.task.receipt.open",
            "对待办的改动会留下动作记录，或标成失败。",
            "Task mutations are recorded with an Action Ledger receipt or a failure state."
        );
        tr!(
            "console.task.summary.closed",
            "已关掉的待办，留给记录和历史。",
            "Closed task kept for receipt and history."
        );
        tr!(
            "console.task.summary.private",
            "私密{0}待办。标题和正文不在这张列表里；打开详情再看。",
            "Private {0} task. Task title and body stay private in this list; open details to review them."
        );
        tr!(
            "console.task.summary.tracking",
            "跟进中。我会把它留在视线里，有进展就提醒你。",
            "Tracking item. Aegis is keeping this visible as a follow-up loop."
        );
        tr!(
            "console.task.summary.scheduled",
            "定时待办。到点我会提醒、检查或继续。",
            "Scheduled task. Aegis will notify, check, or continue it at the right time."
        );
        tr!(
            "console.task.summary.backlog",
            "以后再说。除非你把它拉回来，否则先安静放着。",
            "Backlog item. It is intentionally quiet unless you pull it forward."
        );
        tr!(
            "console.task.summary.decision",
            "要你选的。我把它留着，等你选或再看一眼。",
            "Decision item. Aegis is preserving this as something to choose or review."
        );
        tr!(
            "console.task.summary.notes",
            "想看备注、时间和历史时再打开。",
            "Open when you want notes, timing, and history."
        );
        tr!(
            "console.task.summary.follow_up",
            "跟进中。{0}",
            "Follow-up loop. {0}"
        );
        tr!(
            "console.task.summary.recovery",
            "待办需要你帮忙恢复。打开详情看失败原因，再选下一步。",
            "Task needs recovery. Open details to inspect the failure and choose the next safe step."
        );
        tr!(
            "console.task.summary.priority",
            "{0}优先待办。{1}",
            "{0}-priority task. {1}"
        );
        tr!(
            "console.task.summary.review",
            "待你看的待办。{0}",
            "Task to review. {0}"
        );
        tr!(
            "console.task.mode.scheduled",
            "除非你改它，到点我会再拿出来。",
            "Aegis will surface it again at the scheduled time unless you change it."
        );
        tr!(
            "console.task.mode.waiting",
            "在等你开口，我才改任何东西。",
            "It is waiting for owner input before Aegis changes anything."
        );
        tr!(
            "console.task.mode.notes",
            "想看时间、备注和历史时再打开。",
            "Open when you want timing, notes, and history."
        );
        tr!(
            "console.task.reason.closed",
            "留给历史和记录核对。",
            "Kept for history and receipt review."
        );
        tr!(
            "console.task.reason.tracking",
            "这件事还没了结，所以我一直放在你能看见的地方。",
            "Aegis is tracking this because it is part of an open follow-up loop."
        );
        tr!(
            "console.task.reason.scheduled",
            "我按约定的检查或提醒时间先拿着。",
            "Aegis is holding this for its scheduled check or notification time."
        );
        tr!(
            "console.task.reason.backlog",
            "先安静放着，等你把它拉回来。",
            "This is intentionally quiet until you pull it forward."
        );
        tr!(
            "console.task.reason.decision",
            "要你选一下，所以我留着。",
            "Aegis is preserving this because it needs an owner choice."
        );
        tr!(
            "console.task.reason.review",
            "这件事拿给你看一眼。",
            "Aegis surfaced this task for owner review."
        );
        tr!(
            "console.task.reason.follow_up",
            "我把它当跟进，不是马上要你拍板。",
            "Aegis is keeping this visible as a follow-up, not as an immediate decision."
        );
        tr!(
            "console.task.reason.recovery",
            "这件待办需要你帮忙恢复。",
            "Aegis needs owner help to recover this task."
        );
        tr!(
            "console.task.reason.active",
            "这是你正在进行的事，所以拿出来了。",
            "Aegis surfaced this task because it is active owner work."
        );
        tr!("console.task.title.closed", "已关掉的待办", "Closed task");
        tr!("console.task.title.tracking", "跟进中", "Tracking item");
        tr!("console.task.title.scheduled", "定时待办", "Scheduled task");
        tr!("console.task.title.backlog", "以后再说", "Backlog item");
        tr!("console.task.title.decision", "要你选的", "Decision item");
        tr!("console.task.title.urgent", "紧急待办", "Urgent task");
        tr!(
            "console.task.title.high",
            "高优先级待办",
            "High-priority task"
        );
        tr!(
            "console.task.title.recovery",
            "待办需要恢复",
            "Task needs recovery"
        );
        tr!(
            "console.task.title.review",
            "待你看的待办",
            "Task to review"
        );
        tr!(
            "console.task.title.closed_private",
            "已关掉的私密待办",
            "Closed private task"
        );
        tr!("console.task.title.private", "私密{0}", "Private {0}");
        tr!("console.task.priority.urgent", "紧急", "Urgent");
        tr!("console.task.priority.high", "高", "High");
        tr!("console.task.priority.low", "低", "Low");
        tr!("console.task.priority.normal", "普通", "Normal");
        tr!("console.task.priority.private.urgent", "紧急", "urgent");
        tr!(
            "console.task.priority.private.high",
            "高优先级",
            "high-priority"
        );
        tr!(
            "console.task.priority.private.low",
            "低优先级",
            "low-priority"
        );
        tr!("console.task.priority.private.normal", "普通", "normal");

        tr!("console.memory.action.keep", "继续用", "Keep using");
        tr!("console.memory.action.unpin", "取消钉住", "Unpin");
        tr!("console.memory.action.pin", "钉住", "Pin");
        tr!("console.memory.action.wrong", "标成错的", "Mark wrong");
        tr!("console.memory.action.forget", "忘掉", "Forget");
        tr!(
            "console.memory.action.review",
            "看记忆详情",
            "Review memory detail"
        );
        tr!(
            "console.memory.title",
            "记忆复查：{0}",
            "Memory review: {0}"
        );
        tr!(
            "console.memory.summary",
            "这条钉住的记忆已经 {0} 天没核对。继续依赖前，请你看一眼。",
            "This pinned memory has not been checked in {0} days. Review it before Aegis keeps relying on it."
        );
        tr!(
            "console.memory.reason",
            "钉住的长期记忆够旧了，偶尔该你看一眼。",
            "A pinned durable memory is old enough to deserve occasional owner review."
        );
        tr!(
            "console.memory.next",
            "继续用、钉住、标成错的，或忘掉。需要改写就叫我。",
            "Keep using, pin, mark wrong, or forget this memory. Ask Aegis if it needs rewriting."
        );
        tr!(
            "console.memory.blocked",
            "原始记忆细节仍在详情门后面。",
            "Raw memory detail stays behind the memory detail gate."
        );
        tr!(
            "console.memory.receipt",
            "改正、删除和钉住都会留下记录或更新结果。",
            "Memory corrections, deletes, and pins are recorded with a ledger receipt or update result."
        );

        tr!("console.goal.close", "结束跟进", "Close follow-up");
        tr!(
            "console.goal.closed",
            "已结束目标跟进，原有进度和历史记录已保留。",
            "Goal follow-up closed. Existing progress and history have been kept."
        );
        tr!("console.goal.fallback", "进行中的目标", "Active goal");
        tr!(
            "console.goal.no_milestones",
            "还没有里程碑清单。",
            "No milestone checklist is attached."
        );
        tr!(
            "console.goal.milestones",
            "{0}/{1} 个里程碑完成。",
            "{0}/{1} milestones complete."
        );
        tr!("console.goal.progress", "进度 {0}%。", "{0}% progress.");
        tr!("console.goal.target", " 目标日：{0}。", " Target: {0}.");
        tr!(
            "console.goal.reason",
            "这个进行中的目标是你正在做的一部分。",
            "This active goal is part of the owner's ongoing work."
        );
        tr!(
            "console.goal.next",
            "问我下一步具体做什么，或把它拆成待办。",
            "Ask Aegis for the next concrete step or to turn this into tasks."
        );
        tr!(
            "console.goal.receipt",
            "目标有变，应先写成明确的待办、决定或记录，再声称完成。",
            "Goal changes should be recorded as explicit tasks, decisions, or receipts before completion claims."
        );
        tr!(
            "console.route.approval",
            "这份批准在等你看。",
            "This approval is waiting for your review."
        );
        tr!(
            "console.route.task_paused",
            "暂停的自动操作在等你选择。",
            "Paused automation is waiting for your choice."
        );
        tr!(
            "console.route.task_queue",
            "这件待办来自你的待办列表。",
            "This task came from your task list."
        );
        tr!(
            "console.route.memory",
            "这条记忆可能要你看一眼。",
            "This memory may need your review."
        );
        tr!(
            "console.route.goal",
            "这是你列表里还在进行的目标。",
            "This is an active goal from your list."
        );
        tr!(
            "console.route.waiting",
            "这条待确认的知识在等你看。",
            "This waiting knowledge item is here for your review."
        );
        tr!(
            "console.route.receipt",
            "有一条记录可以查看。",
            "A receipt is ready to review."
        );
        tr!(
            "console.route.system",
            "这是一条来源是否可用的通知。",
            "This is a notice about whether a source is available."
        );

        tr!(
            "console.waiting.dependency",
            "依赖的那件事",
            "the dependency"
        );
        tr!("console.waiting.due", " 截止 {0}。", " Due {0}.");
        tr!(
            "console.waiting.summary.on",
            "在等{0}。{1}",
            "Waiting on {0}.{1}"
        );
        tr!(
            "console.waiting.summary.due",
            "有截止日期的在等之事。{0}",
            "Waiting item with a due date.{0}"
        );
        tr!("console.waiting.reason.on", "在等{0}。", "Waiting on {0}.");
        tr!(
            "console.waiting.reason.due",
            "它有截止日期，所以我一直放在你能看见的地方。",
            "Aegis is keeping this visible because it has a due date."
        );
        tr!(
            "console.waiting.next.on",
            "跟进{0}，推迟，或让我结束这件事。",
            "Follow up about {0}, defer it, or ask Aegis to clean up this loop."
        );
        tr!(
            "console.waiting.next.due",
            "看看这件有日期的跟进还要不要，然后推迟或关掉。",
            "Review whether this dated follow-up is still needed, then defer or close it."
        );
        tr!("console.waiting.title", "在等的事", "Waiting item");
        tr!("console.waiting.action.done", "完成", "Done");
        tr!(
            "console.waiting.action.tomorrow",
            "明天再看",
            "Review tomorrow"
        );
        tr!(
            "console.waiting.receipt",
            "跟进动作应先变成待办、草稿或记录，我才声称做完。",
            "Follow-up actions should produce a task, draft, or receipt before Aegis claims completion."
        );

        tr!(
            "console.receipt.blocked.draft",
            "这份草稿还不能执行：确认路径和记录路径还没接上。",
            "Execution is intentionally disabled for this draft until an approval route and receipt path are connected."
        );
        tr!(
            "console.receipt.blocked.missing",
            "还没有像记录一样的证据，完成不能算数。",
            "Completion is not trusted until receipt-like evidence exists."
        );
        tr!(
            "console.receipt.reason.draft",
            "我按你的话起草了，保存或执行前要你确认。",
            "Aegis drafted this from your instruction and needs your confirmation before saving or executing anything."
        );
        tr!(
            "console.receipt.reason.inspect",
            "有一件动作处于进行、完成、失败或待核记录的状态。",
            "An action has a running, completed, failed, or receipt state to inspect."
        );
        tr!(
            "console.receipt.action.review_draft",
            "看草稿",
            "Review draft"
        );
        tr!("console.receipt.action.open", "打开记录", "Open receipt");
        tr!(
            "console.receipt.next.reminder",
            "看看提醒文字。时间和措辞都对再保存；否则取消或让我改。",
            "Review the reminder text. Save it only if the timing and wording are right; otherwise cancel or ask Aegis to revise it."
        );
        tr!(
            "console.receipt.next.task",
            "看看待办文字。它属于你的工作队列再保存；否则取消或让我改。",
            "Review the task text. Save it only if it belongs in your work queue; otherwise cancel or ask Aegis to revise it."
        );
        tr!(
            "console.receipt.next.draft",
            "看看草稿。还没执行；确认和记录接上之前，执行仍被挡住。",
            "Review the draft. Nothing has been executed, and execution remains blocked until approval and receipt handling are connected."
        );
        tr!(
            "console.receipt.next.missing",
            "打开动作记录，核对为什么还缺证据。",
            "Open the action record and verify why receipt evidence is missing."
        );
        tr!(
            "console.receipt.next.verify",
            "打开记录，核对发生了什么。",
            "Open the receipt to verify what happened."
        );
        tr!(
            "console.receipt.expect.draft",
            "还没有对外做过任何事。这份草稿以后若执行，必须先有耐久记录，我才能声称做完。",
            "No external action has run. If this draft is later executed, a durable receipt is required before Aegis can claim completion."
        );
        tr!(
            "console.receipt.expect.missing",
            "还缺像记录一样的证据，这件事不能算完成。",
            "Receipt-like evidence is still required before this action counts as complete."
        );
        tr!(
            "console.receipt.expect.verify",
            "打开记录，核对我为这次动作记下的证据。",
            "Open the receipt to verify the evidence Aegis recorded for this action."
        );
        tr!(
            "console.receipt.draft.title.reminder",
            "看看提醒：{0}",
            "Review reminder: {0}"
        );
        tr!(
            "console.receipt.draft.title.task",
            "看看待办：{0}",
            "Review task: {0}"
        );
        tr!(
            "console.receipt.draft.title.tracking",
            "看看跟进：{0}",
            "Review tracking task: {0}"
        );
        tr!(
            "console.receipt.draft.title.backlog",
            "看看以后再说：{0}",
            "Review backlog item: {0}"
        );
        tr!(
            "console.receipt.draft.title.decision",
            "看看要你选的：{0}",
            "Review decision item: {0}"
        );
        tr!(
            "console.receipt.draft.title.generic",
            "看看草稿：{0}",
            "Review draft: {0}"
        );
        tr!(
            "console.receipt.draft.title.email",
            "看看邮件草稿",
            "Review email draft"
        );
        tr!("console.receipt.draft.fallback", "草稿", "draft");
        tr!(
            "console.receipt.draft.summary.email",
            "邮件草稿已留下供你看。还没有发出去。",
            "Email draft is saved for review. Nothing has been executed."
        );
        tr!(
            "console.receipt.draft.summary.reminder",
            "保存前先看这条拟定提醒。还没有保存或发出。",
            "Review the proposed reminder before Aegis saves it. Nothing has been saved or sent yet."
        );
        tr!(
            "console.receipt.draft.summary.task",
            "保存前先看这条拟定待办。还没有保存或执行。",
            "Review the proposed task before Aegis saves it. Nothing has been saved or executed yet."
        );
        tr!(
            "console.receipt.draft.summary.tracking",
            "保存前先看这条拟定跟进。还没有保存或执行。",
            "Review the proposed tracking item before Aegis saves it. Nothing has been saved or executed yet."
        );
        tr!(
            "console.receipt.draft.summary.backlog",
            "保存前先看这条拟定的以后再说。还没有保存或执行。",
            "Review the proposed backlog item before Aegis saves it. Nothing has been saved or executed yet."
        );
        tr!(
            "console.receipt.draft.summary.decision",
            "保存前先看这条拟定的要你选的。还没有保存或执行。",
            "Review the proposed decision item before Aegis saves it. Nothing has been saved or executed yet."
        );
        tr!(
            "console.receipt.draft.summary.generic",
            "看看这条拟定的{0}。还没有执行。",
            "Review the proposed {0}. Nothing has been executed yet."
        );
        tr!(
            "console.receipt.summary.proposed",
            "{0}已留下供你看。还没有执行。",
            "{0} is saved for review. Nothing has been executed."
        );
        tr!(
            "console.receipt.summary.pending",
            "{0}在等你确认。有记录之前，不要当成做完。",
            "{0} is waiting for approval. Nothing should be treated as complete until a receipt exists."
        );
        tr!(
            "console.receipt.summary.approved",
            "{0}已同意但还没做完。执行后仍要有记录。",
            "{0} is approved but not complete. A receipt is still required after execution."
        );
        tr!(
            "console.receipt.summary.state",
            "{0}。{1}。打开核对发生了什么。",
            "{0} action. {1}. Open it to verify what happened."
        );
        tr!(
            "console.receipt.detail.available",
            "已有记录",
            "Receipt available"
        );
        tr!(
            "console.receipt.detail.needed",
            "还缺记录",
            "Receipt still needed"
        );
        tr!("console.receipt.status.completed", "已完成", "Completed");
        tr!("console.receipt.status.executing", "进行中", "Executing");
        tr!(
            "console.receipt.status.pending",
            "待确认",
            "Pending approval"
        );
        tr!("console.receipt.status.proposed", "已拟定", "Proposed");
        tr!("console.receipt.status.failed", "没做成", "Failed");
        tr!(
            "console.receipt.status.needed",
            "还缺记录",
            "Receipt needed"
        );
        tr!("console.receipt.status.action", "动作", "Action");
        tr!(
            "console.receipt.title.reminder_sent",
            "已发出提醒：{0}",
            "Reminder sent: {0}"
        );
        tr!(
            "console.receipt.title.notice_sent",
            "已发出通知：{0}",
            "Notice sent: {0}"
        );
        tr!(
            "console.receipt.title.email_sent",
            "已发出邮件：{0}",
            "Email sent: {0}"
        );
        tr!(
            "console.receipt.title.task_saved",
            "已记下待办：{0}",
            "Task saved: {0}"
        );
        tr!(
            "console.receipt.title.calendar_added",
            "已加到日历：{0}",
            "Added to calendar: {0}"
        );
        tr!("console.receipt.title.done", "已完成：{0}", "Done: {0}");
        tr!(
            "console.receipt.title.failed",
            "没做成：{0}",
            "Didn't finish: {0}"
        );
        tr!(
            "console.receipt.title.running",
            "正在做：{0}",
            "Working on: {0}"
        );
        tr!(
            "console.receipt.title.verify",
            "核对记录：{0}",
            "Check the record: {0}"
        );
        tr!("console.receipt.title.review", "看看{0}", "Review {0}");
        tr!("console.receipt.title.approve", "确认{0}", "Approve {0}");
        tr!(
            "console.receipt.title.approved",
            "已同意{0}",
            "Approved {0}"
        );
        tr!("console.receipt.untitled", "这件事", "this action");
        tr!("console.receipt.kind.reminder", "提醒", "reminder");
        tr!("console.receipt.kind.notice", "通知", "notice");
        tr!("console.receipt.kind.email", "邮件", "email");
        tr!(
            "console.receipt.kind.email_draft",
            "邮件草稿",
            "email draft"
        );
        tr!("console.receipt.kind.task", "待办", "task");
        tr!("console.receipt.kind.calendar", "日历", "calendar");
        tr!("console.receipt.kind.memory", "记忆", "memory");
        tr!("console.receipt.kind.generic", "动作", "action");
        tr!(
            "console.receipt.kind.draft_action",
            "草稿动作",
            "draft action"
        );

        tr!(
            "console.system.title.unavailable",
            "记录暂时看不到",
            "Receipts unavailable"
        );
        tr!(
            "console.system.summary.no_db",
            "动作记录需要持久数据库；这次没有列出任何记录行。",
            "Action Ledger requires a persistent database; no receipt rows were projected."
        );
        tr!(
            "console.system.summary.query_failed",
            "动作记录查不到；记录行被省略了。",
            "Action Ledger could not be queried; receipt rows were omitted."
        );
        tr!(
            "console.system.reason",
            "系统健康或来源可用性有变化。",
            "System health or source availability changed."
        );
        tr!(
            "console.system.next",
            "只有来源或系统状态需要注意时，再看。",
            "Review only if the source or system state needs attention."
        );
        tr!(
            "console.system.receipt",
            "除非你接着做，否则不需要对外记录。",
            "No external receipt is required unless you take a follow-up action."
        );
        tr!(
            "console.action.completed",
            "动作完成了，我记下了结果。",
            "Action completed and Aegis recorded the result."
        );

        tr!(
            "console.mfa.memory",
            "先完成最近一次身份核验，再拉取记忆详情。",
            "Complete recent MFA before memory details are fetched."
        );
        tr!(
            "console.mfa.profile",
            "先完成最近一次身份核验，再拉取档案详情。",
            "Complete recent MFA before profile slot details are fetched."
        );
        tr!(
            "console.mfa.task",
            "先完成最近一次身份核验，再拉取待办详情。",
            "Complete recent MFA before task details are fetched."
        );
        tr!(
            "console.mfa.receipt",
            "先完成最近一次身份核验，再拉取动作记录详情。",
            "Complete recent MFA before action receipt details are fetched."
        );
        tr!(
            "console.memory.confirmed",
            "记忆已确认。我可以继续用。",
            "Memory confirmed. Aegis can keep using it."
        );
        tr!(
            "console.memory.marked_wrong",
            "你在工作台标成错的。原文已隐藏。",
            "Marked wrong by owner in Workbench. Original fact hidden."
        );
        tr!(
            "console.mandate.saved",
            "规则已保存。我可以盯着对得上的条目并准备复查卡片；私人细节、提醒和对外动作仍会先问你，并留下记录。",
            "Rule saved. Aegis can watch matching items and prepare review cards; private details, reminders, and outside actions still ask first and keep receipts."
        );
        tr!(
            "console.principal.telegram",
            "Telegram 用户 {0}",
            "Telegram user {0}"
        );

        tr!(
            "console.brief.daily.title",
            "{0} 的今日摘要（{1}）",
            "Daily Brief for {0} ({1})"
        );
        tr!(
            "console.brief.evening.title",
            "{0} 的晚间回顾（{1}）",
            "Evening Review for {0} ({1})"
        );
        tr!("console.brief.generated", "生成于：", "Generated at: ");
        tr!("console.brief.warnings", "提醒", "Warnings");
        tr!("console.brief.section.calendar", "日历", "Calendar");
        tr!(
            "console.brief.section.tasks",
            "进行中的待办",
            "Active Tasks"
        );
        tr!("console.brief.section.waiting", "在等", "Waiting For");
        tr!(
            "console.brief.section.approvals",
            "待你确认",
            "Pending Approvals"
        );
        tr!("console.brief.section.alerts", "提醒", "Alerts");
        tr!(
            "console.brief.section.actions",
            "最近动作",
            "Recent Actions"
        );
        tr!(
            "console.brief.section.suggested",
            "建议动作",
            "Suggested Actions"
        );
        tr!("console.brief.section.evening_actions", "动作", "Actions");
        tr!("console.brief.section.pending", "仍未完成", "Still Pending");
        tr!("console.brief.section.facts", "记住的事实", "Durable Facts");
        tr!(
            "console.brief.calendar.none",
            "已问过日历；这次没有事件。",
            "Calendar queried; no events were provided."
        );
        tr!(
            "console.brief.calendar.not_queried",
            "没去问日历；不要当成没有事件。",
            "Calendar not queried; do not assume there are no events."
        );
        tr!(
            "console.brief.calendar.unavailable",
            "日历不可用；不要当成没有事件。",
            "Calendar unavailable; do not assume there are no events."
        );
        tr!("console.brief.calendar.at", " 在 {0}", " at {0}");
        tr!("console.brief.calendar.location", "（{0}）", " ({0})");
        tr!("console.brief.detail", "{0} 详情：{1}", "{0} Detail: {1}");
        tr!(
            "console.brief.tasks.empty",
            "没有提供进行中的待办。",
            "No active tasks provided."
        );
        tr!("console.brief.tasks.due", " 截止 {0}", " due {0}");
        tr!(
            "console.brief.waiting.empty",
            "没有提供在等的事。",
            "No waiting-for items provided."
        );
        tr!(
            "console.brief.waiting.on",
            "{0} —— 在等 {1}",
            "{0} -- waiting on {1}"
        );
        tr!(
            "console.brief.approvals.empty",
            "没有提供待确认。",
            "No pending approvals provided."
        );
        tr!(
            "console.brief.approvals.by",
            "{0} —— 由 {1} 提出",
            "{0} -- requested by {1}"
        );
        tr!(
            "console.brief.alerts.empty",
            "没有提供未处理提醒。",
            "No unresolved alerts provided."
        );
        tr!("console.brief.alerts.severity", "{0} [{1}]", "{0} [{1}]");
        tr!(
            "console.brief.actions.empty",
            "没有提供动作记录。",
            "No action ledger entries provided."
        );
        tr!("console.brief.actions.line", "{0}：{1}", "{0}: {1}");
        tr!(
            "console.brief.actions.receipt",
            "（记录：{0}）",
            " (receipt: {0})"
        );
        tr!(
            "console.brief.actions.receipt_missing",
            "（还缺记录）",
            " (receipt missing)"
        );
        tr!(
            "console.brief.actions.error",
            "（错误：{0}）",
            " (error: {0})"
        );
        tr!(
            "console.brief.suggested.empty",
            "没有提供建议动作。",
            "No suggested actions provided."
        );
        tr!(
            "console.brief.pending.empty",
            "没有提供未完成的事。",
            "No pending items provided."
        );
        tr!(
            "console.brief.facts.empty",
            "没有提供新的长期事实。",
            "No new durable facts provided."
        );
        tr!("console.brief.action.proposed", "已拟定", "Proposed");
        tr!("console.brief.action.pending", "待确认", "Pending approval");
        tr!("console.brief.action.executing", "进行中", "Executing");
        tr!("console.brief.action.executed", "已执行", "Executed");
        tr!("console.brief.action.failed", "没做成", "Failed");
        tr!("console.brief.action.skipped", "已跳过", "Skipped");
        tr!("console.brief.action.cancelled", "已取消", "Cancelled");
        tr!("console.brief.action.unknown", "未知状态", "Unknown status");
        tr!(
            "console.model.credits",
            "模型额度用完了。这次没动手。",
            "Aegis model provider credits are exhausted. No action was taken."
        );
        tr!(
            "console.model.grok_missing",
            "Grok 模型现在不可用。换成支持的 xAI 模型，或聊天先别用 Grok，再试一次。",
            "Grok model is not available. Switch to a supported xAI model or stop using Grok for chat, then retry."
        );
        tr!(
            "console.model.grok_shape",
            "Grok 拒了这次请求形状。要先修兼容性，才能再走这条路。",
            "Grok rejected this request shape. Aegis needs a provider compatibility fix before retrying this path."
        );
        tr!(
            "console.model.unknown",
            "这个模型不在当前服务商的列表里。",
            "That model is not in the current provider's catalog."
        );
        tr!(
            "console.model.choose",
            "请先在设置里选一个模型。",
            "Choose a model in Settings first."
        );
        tr!(
            "console.tasks.receipt",
            "只读待办列表。没有对外做过任何事。",
            "Read-only task list. No external action was executed."
        );
        tr!(
            "console.tasks.source.label",
            "待办和提醒",
            "Tasks and reminders"
        );
        tr!(
            "console.tasks.source.summary",
            "读了进行中的待办/提醒记录。",
            "Active task/reminder records were read."
        );
        tr!(
            "console.tasks.source.detail",
            "我直接从待办库回答了 /tasks，没有问模型，也没有打开旧的 Telegram 面板。",
            "Aegis answered /tasks from the task store directly instead of asking the model or opening the legacy Telegram panel."
        );
        tr!(
            "console.push.delivered",
            "已把网页通知发到 {0}/{1} 台已保存设备。",
            "Delivered WebApp notification to {0}/{1} saved device(s)."
        );
        tr!(
            "console.push.cleaned",
            "网页通知没发出去；过期设备订阅已清理。",
            "No WebApp notification was delivered; expired device subscriptions were cleaned up."
        );
        tr!(
            "console.push.failed",
            "网页通知没发出去。先留着 Telegram 兜底，检查设备通知权限后再试。",
            "No WebApp notification was delivered. Keep Telegram fallback on and retry after checking device notification permission."
        );

        // Console provider status (Grok / Codex) — owner-facing, Chinese-first.
        tr!(
            "console.provider.grok.needs_signin",
            "Grok 还没登录。",
            "Grok is not signed in yet."
        );
        tr!(
            "console.provider.grok.available",
            "Grok 还没连接。",
            "Grok is not connected."
        );
        tr!(
            "console.provider.grok.active",
            "Grok 已连接，正在用于聊天。",
            "Grok is connected and used for chat."
        );
        tr!(
            "console.provider.grok.connected",
            "Grok 已连接。",
            "Grok is connected."
        );
        tr!(
            "console.provider.grok.pending",
            "需要你在 xAI 页面输入验证码。",
            "Enter the code on the xAI page."
        );
        tr!(
            "console.provider.grok.next.connect",
            "连接后可用于聊天。",
            "Connect to use it for chat."
        );
        tr!(
            "console.provider.grok.next.use",
            "点「用于聊天」即可。",
            "Tap Use for chat."
        );
        tr!(
            "console.provider.grok.next.enter_code",
            "在 xAI 页面输入验证码，然后点「我已批准」。",
            "Enter the code on the xAI page, then tap I approved it."
        );
        tr!(
            "console.provider.codex.active",
            "Codex 已连接，正在用于聊天。",
            "Codex is connected and used for chat."
        );
        tr!(
            "console.provider.codex.connected",
            "Codex 已连接。",
            "Codex is connected."
        );
        tr!(
            "console.provider.codex.api_key",
            "Codex 用的是 API 密钥，不能走订阅聊天。",
            "Codex is using an API key, so it cannot use subscription chat."
        );
        tr!(
            "console.provider.codex.not_logged_in",
            "Codex 还没登录。",
            "Codex is not signed in yet."
        );
        tr!(
            "console.provider.codex.not_installed",
            "这台机器还没装 Codex。",
            "Codex is not installed on this machine."
        );
        tr!(
            "console.provider.codex.next.use",
            "点「用于聊天」即可。",
            "Tap Use for chat."
        );
        tr!(
            "console.provider.codex.next.signin",
            "点「连接」，在 OpenAI 页面输入验证码。",
            "Tap Connect, then enter the code on the OpenAI page."
        );

        m
    });

/// Look up a translated string. Falls back to English, then returns the key itself.
///
/// Normalizes locale by stripping region suffix (e.g. "zh-CN" -> "zh", "en-US" -> "en")
/// so that settings using "zh-CN" match translation keys stored under "zh".
pub fn t(key: &str, locale: &str) -> &'static str {
    // Normalize locale: strip region suffix (e.g. "zh-CN" -> "zh")
    let base_locale = locale.split('-').next().unwrap_or(locale);

    TRANSLATIONS
        .get(key)
        .and_then(|locales| locales.get(base_locale).or_else(|| locales.get("en")))
        .copied()
        .unwrap_or("[MISSING]")
}

/// Format a translated string with one argument (simple {} replacement)
pub fn tf(key: &str, locale: &str, arg: &str) -> String {
    t(key, locale).replace("{}", arg)
}

/// Format a translated string with multiple arguments (replaces {0}, {1}, {2}, ...)
pub fn tfn(key: &str, locale: &str, args: &[&str]) -> String {
    let mut result = t(key, locale).to_string();
    for (i, arg) in args.iter().enumerate() {
        result = result.replacen(&format!("{{{}}}", i), arg, 1);
    }
    // Also replace bare {} sequentially for backwards-compat
    for arg in args {
        if let Some(pos) = result.find("{}") {
            result.replace_range(pos..pos + 2, arg);
        }
    }
    result
}

/// Single-pass positional template fill: replaces `{0}`,`{1}`,… in the localized
/// template left-to-right in ONE pass. Unlike [`tfn`], inserted argument values
/// are NEVER re-scanned, so an argument containing a literal `{}` / `{1}` cannot
/// be clobbered by a later token. Use this whenever an ARBITRARY string (error
/// text, email recipient/subject, user-authored title) is interpolated — it is
/// immune to the bare-`{}` / cross-token re-scan corruption class that bit `tfn`.
/// Unknown `{N}` tokens (no matching arg) are dropped; a `{` not part of a
/// `{<digits>}` token is emitted literally.
pub fn tfp(key: &str, locale: &str, args: &[&str]) -> String {
    let template = t(key, locale);
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    loop {
        match rest.find('{') {
            None => {
                out.push_str(rest);
                break;
            }
            Some(b) => {
                let after = &rest[b + 1..];
                let digits_end = after
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(after.len());
                if digits_end > 0 && after[digits_end..].starts_with('}') {
                    // `{<digits>}` token — emit preceding text, then the arg (NOT re-scanned).
                    out.push_str(&rest[..b]);
                    if let Ok(n) = after[..digits_end].parse::<usize>()
                        && let Some(arg) = args.get(n)
                    {
                        out.push_str(arg);
                    }
                    rest = &after[digits_end + 1..];
                } else {
                    // Literal `{` (not a positional token) — emit up to and including it.
                    out.push_str(&rest[..b + 1]);
                    rest = &rest[b + 1..];
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zh_translation() {
        assert_eq!(t("tab.skills", "zh"), "技能状态");
        assert_eq!(t("tab.review", "zh"), "审查统计");
        assert_eq!(
            t("console.source.status.delegated", "zh"),
            "Codex 访问待验证"
        );
        assert_eq!(
            t("console.settings.calendar.delegated", "zh"),
            "日历读取需要 Codex 连接器；执行前会验证访问权限。"
        );
        assert_eq!(
            t("settings.audit.fallback_authorized_without_model", "zh"),
            "备用模型已授权，但还没选备用模型：主模型出问题时不会自动切换。"
        );
    }

    #[test]
    fn test_en_translation() {
        assert_eq!(t("tab.skills", "en"), "Skills");
        assert_eq!(t("tab.review", "en"), "Review Guard");
        assert_eq!(
            t("console.source.status.delegated", "en"),
            "Codex access not yet verified"
        );
        assert_eq!(
            t("console.settings.calendar.delegated", "en"),
            "Calendar reads require a Codex connector; access is verified before execution."
        );
        assert_eq!(
            t("settings.audit.fallback_authorized_without_model", "en"),
            "Backup model is authorized, but no backup model is selected: chat will not switch automatically when the primary fails."
        );
    }

    #[test]
    fn test_fallback_to_english() {
        assert_eq!(t("tab.skills", "fr"), "Skills"); // unknown locale falls back to en
    }

    #[test]
    fn test_missing_key_returns_missing() {
        assert_eq!(t("nonexistent.key", "zh"), "[MISSING]");
    }

    #[test]
    fn test_all_keys_have_both_locales() {
        for (key, locales) in TRANSLATIONS.iter() {
            assert!(
                locales.contains_key("zh"),
                "Key '{}' missing Chinese translation",
                key
            );
            assert!(
                locales.contains_key("en"),
                "Key '{}' missing English translation",
                key
            );
        }
    }

    /// i18n Task 2: zh/en parity guard for the backend catalog. Collects every
    /// gap into a Vec so a failure reports ALL missing entries at once (a missing
    /// translation must fail the test, never silently leak as an English/`[MISSING]`
    /// fallback — see spec §3.4).
    #[test]
    fn every_key_has_zh_and_en() {
        let mut missing: Vec<String> = Vec::new();
        for (key, locales) in TRANSLATIONS.iter() {
            for want in ["zh", "en"] {
                if !locales.contains_key(want) {
                    missing.push(format!("{key} missing {want}"));
                }
            }
        }
        assert!(missing.is_empty(), "catalog gaps:\n{}", missing.join("\n"));
    }

    /// Regression guard (cto-review of the HITL Task-3 batch): the three HITL keys
    /// that get an ARBITRARY string (shell command / error text) appended must be
    /// PREFIX-ONLY — they carry only safe numeric/identifier args via `tfn`, and the
    /// arbitrary string is concatenated AFTER. Routing an arbitrary string through
    /// `tfn` re-introduces the bare-`{}` re-scan corruption (the 694f6f39 bug class):
    /// a command like `find . -exec rm {} \;` or a `serde error at {} token` would be
    /// clobbered. This asserts each prefix has no leftover `{` and a literal `{}` in
    /// the appended part survives intact.
    #[test]
    fn tfp_single_pass_no_cross_token_or_rescan() {
        // tfp must fill {0}/{1}/{2} in ONE pass without re-scanning inserted values.
        // config.exec.changed_from_to interpolates arbitrary key/old/new; a
        // value carrying a literal positional token or `{}` must NOT clobber a
        // later field (the chained-replacen cross-token bug this replaced).
        let sent = tfp(
            "config.exec.changed_from_to",
            "en",
            &["evil{1}@x.com", "RealSubject", "MID1"],
        );
        assert!(
            sent.contains("evil{1}@x.com"),
            "key literal token corrupted: {sent}"
        );
        assert!(
            sent.contains("RealSubject"),
            "old value lost to cross-token: {sent}"
        );
        assert!(sent.contains("MID1"), "new value lost: {sent}");
        // Literal `{}` and `{2}` inside a value survive (no re-scan), both locales.
        for loc in ["zh", "en"] {
            let s = tfp(
                "config.exec.changed_from_to",
                loc,
                &["x@x.com", "a {} b {2} c", "MID2"],
            );
            assert!(
                s.contains("a {} b {2} c"),
                "value braces corrupted ({loc}): {s}"
            );
            assert!(s.contains("MID2"), "new value lost ({loc}): {s}");
        }
        // Unknown token (no matching arg) is dropped, not left dangling.
        assert!(!tfp("config.exec.changed_from_to", "en", &["only-to"]).contains("{1}"));

        // tfp keys carrying ARBITRARY args (config key/old/new, count). Each
        // arbitrary value embeds a literal positional token / bare `{}` that a
        // tf/tfn re-scan would clobber; tfp must preserve it verbatim, both locales.
        for loc in ["zh", "en"] {
            // panel.action.cleared_finished: {0}=count is routed through tfp.
            // count is digits-only in production, but a value carrying a literal
            // positional token must still survive single-pass fill (no re-scan).
            let cf = tfp("panel.action.cleared_finished", loc, &["3 {0} {}"]);
            assert!(
                cf.contains("3 {0} {}"),
                "cleared_finished count corrupted ({loc}): {cf}"
            );

            // ── apply_approved_config (ConfigApplyResult::localized) keys ──
            // config.exec.updated_to: {0}=key {1}=value, BOTH arbitrary.
            let ut = tfp("config.exec.updated_to", loc, &["k {0}", "v {1} {}"]);
            assert!(ut.contains("k {0}"), "exec.updated_to key ({loc}): {ut}");
            assert!(ut.contains("v {1} {}"), "exec.updated_to val ({loc}): {ut}");

            // config.exec.unchanged + invalid_payload: {0}=key arbitrary, fixed suffix.
            for key in ["config.exec.unchanged", "config.exec.invalid_payload"] {
                let out = tfp(key, loc, &["k {0} {}"]);
                assert!(out.contains("k {0} {}"), "{key} key ({loc}): {out}");
            }

            // config.exec.changed_from_to: {0}=key {1}=old {2}=new, ALL arbitrary.
            let cft = tfp(
                "config.exec.changed_from_to",
                loc,
                &["k {1}", "old {2}", "new {}"],
            );
            assert!(cft.contains("k {1}"), "exec.changed key ({loc}): {cft}");
            assert!(cft.contains("old {2}"), "exec.changed old ({loc}): {cft}");
            assert!(cft.contains("new {}"), "exec.changed new ({loc}): {cft}");

            // config.exec.hot_reloaded: {0}=from {1}=to, arbitrary.
            let hr = tfp("config.exec.hot_reloaded", loc, &["f {1}", "t {}"]);
            assert!(hr.contains("f {1}"), "exec.hot_reloaded from ({loc}): {hr}");
            assert!(hr.contains("t {}"), "exec.hot_reloaded to ({loc}): {hr}");

            // config.exec.requires_reembed: {0}=old {1}=new {2}=key, ALL arbitrary.
            let rr = tfp(
                "config.exec.requires_reembed",
                loc,
                &["o {1}", "n {2}", "k {}"],
            );
            assert!(rr.contains("o {1}"), "exec.reembed old ({loc}): {rr}");
            assert!(rr.contains("n {2}"), "exec.reembed new ({loc}): {rr}");
            assert!(rr.contains("k {}"), "exec.reembed key ({loc}): {rr}");

            // config.exec.hot_reload_rejected: {0}=reason arbitrary.
            let hrj = tfp("config.exec.hot_reload_rejected", loc, &["why {0} {} x"]);
            assert!(
                hrj.contains("why {0} {} x"),
                "exec.rejected reason ({loc}): {hrj}"
            );

            // config.exec.apply_failed is a tfp PREFIX ({0}=key arbitrary) + concat error.
            let af = tfp("config.exec.apply_failed", loc, &["k {1} y"]);
            assert!(
                af.contains("k {1} y"),
                "exec.apply_failed key ({loc}): {af}"
            );
            let raw_err = "boom {} \\; rm {0}";
            let af_full = format!("{af}{raw_err}");
            assert!(
                af_full.ends_with(raw_err),
                "exec.apply_failed appended error corrupted ({loc}): {af_full}"
            );
        }
    }

    #[test]
    fn hitl_arbitrary_string_keys_are_prefix_only_no_tfn_rescan() {
        for loc in ["zh", "en"] {
            let prefix = tfn("hitl.sysop.progress_with_cmd", loc, &["1", "2", "3"]);
            assert!(
                !prefix.contains('{'),
                "progress_with_cmd prefix still has a placeholder ({loc}): {prefix}"
            );
            let msg = format!("{}{}", prefix, "find . -exec rm {} \\;");
            assert!(
                msg.ends_with("find . -exec rm {} \\;"),
                "command with literal braces corrupted ({loc}): {msg}"
            );

            let p2 = tfn("hitl.exec_failed", loc, &["calendar_create_event"]);
            assert!(
                !p2.contains('{'),
                "exec_failed prefix has placeholder ({loc}): {p2}"
            );
            let m2 = format!("{}{}", p2, "serde error at {} token");
            assert!(
                m2.ends_with("serde error at {} token"),
                "error with literal braces corrupted ({loc}): {m2}"
            );

            // Task 3 additions: revoke error-result keys that carry
            // only a safe identifier via tf and concatenate the arbitrary error
            // (e.to_string()) AFTER. Each template has exactly one `{}` (the
            // identifier), and the appended raw error must survive a literal `{}`.
            {
                let (key, id_arg) = ("hitl.revoke.failed", "work");
                let prefix = tf(key, loc, id_arg);
                assert!(
                    !prefix.contains('{'),
                    "{key} prefix still has a placeholder after tf ({loc}): {prefix}"
                );
                let raw = "boom {} \\; rm {}";
                let msg = format!("{prefix}{raw}");
                assert!(
                    msg.ends_with(raw),
                    "{key} corrupted appended arbitrary string ({loc}): {msg}"
                );
            }

            // PREFIX-only keys: no template arg, then concatenate the
            // arbitrary error AFTER.
            for key in [
                // Panel-button action error results: bare prefix + concat error `e`.
                "panel.action.complete_failed",
                "panel.action.archive_failed",
                "panel.action.skip_failed",
                "panel.action.restore_failed",
                "panel.action.clear_failed",
                "panel.action.delete_failed",
            ] {
                let p = t(key, loc);
                assert!(
                    !p.contains('{'),
                    "{key} bare prefix has placeholder ({loc}): {p}"
                );
                let raw = "err at {} / {0}";
                let m = format!("{p}{raw}");
                assert!(
                    m.ends_with(raw),
                    "{key} corrupted appended arbitrary string ({loc}): {m}"
                );
            }
        }
    }

    #[test]
    fn test_review_guard_labels() {
        assert_eq!(t("review.total", "zh"), "过去24小时审查次数");
        assert_eq!(t("review.total", "en"), "Reviews (24h)");
        assert_eq!(t("layer.fast_pass", "zh"), "快速通过");
        assert_eq!(t("layer.fast_pass", "en"), "Fast Pass");
    }

    #[test]
    fn test_dashboard_translations() {
        assert_eq!(t("dash.title", "zh"), "仪表盘");
        assert_eq!(t("dash.title", "en"), "Dashboard");
        assert_eq!(t("dash.active_skills", "zh"), "活跃技能");
        assert_eq!(t("dash.quick_links", "en"), "Quick Links");
    }

    #[test]
    fn test_meta_panel_translations() {
        assert_eq!(t("meta.title", "zh"), "智能面板");
        assert_eq!(t("meta.title", "en"), "Smart Panels");
        assert_eq!(t("meta.pin", "zh"), "置顶");
        assert_eq!(t("meta.unlock", "en"), "Unlock");
    }

    #[test]
    fn test_task_translations() {
        assert_eq!(t("task.no_active", "zh"), "暂无活跃任务。");
        assert_eq!(t("task.btn_done", "en"), "Done");
        assert_eq!(t("task.priority", "zh"), "优先级");
    }

    #[test]
    fn test_alert_translations() {
        assert_eq!(t("alert.title", "zh"), "告警中心");
        assert_eq!(t("alert.all_clear", "en"), "All systems normal, no alerts.");
    }

    #[test]
    fn test_intent_additions() {
        assert_eq!(t("intent.type_timed", "zh"), "定时");
        assert_eq!(t("intent.mode_llm", "en"), "AI Callback");
        assert_eq!(t("intent.expired", "zh"), "已过期");
    }

    #[test]
    fn test_common_additions() {
        assert_eq!(t("common.no_data", "zh"), "暂无数据");
        assert_eq!(t("common.total", "en"), "Total");
    }

    #[test]
    fn test_locale_normalization() {
        // "zh-CN" should resolve to "zh" translations
        assert_eq!(t("tab.skills", "zh-CN"), "技能状态");
        assert_eq!(t("tab.review", "zh-CN"), "审查统计");
        // "en-US" should resolve to "en" translations
        assert_eq!(t("tab.skills", "en-US"), "Skills");
        assert_eq!(t("tab.review", "en-US"), "Review Guard");
        // "en-GB" should also work
        assert_eq!(t("tab.skills", "en-GB"), "Skills");
    }

    #[test]
    fn test_new_skill_status_keys() {
        assert_eq!(t("skill.status_active", "zh"), "正常");
        assert_eq!(t("skill.status_active", "en"), "Active");
        assert_eq!(t("skill.status_tripped", "zh"), "熔断");
        assert_eq!(t("skill.circuit_alert", "en"), "Circuit breaker tripped");
    }

    #[test]
    fn test_new_approval_keys() {
        assert_eq!(t("approval.approve_once", "zh"), "批准本次");
        assert_eq!(t("approval.approve_once", "en"), "Approve Once");
        assert_eq!(t("approval.trust_domain", "zh"), "信任域名");
    }

    #[test]
    fn test_tfn_sequential_replacement() {
        let result = tfn("hitl.sysop.progress_with_cmd", "en", &["5", "20", "3"]);
        assert!(!result.contains('{'), "all placeholders filled: {result}");
        assert!(result.contains('5') && result.contains("20") && result.contains('3'));
    }

    #[test]
    fn test_tfn_zh_replacement() {
        let result = tfn("hitl.sysop.progress", "zh", &["5", "20", "3"]);
        assert!(result.contains("[5/20]"), "should format as [5/20]");
        assert!(result.contains("(3s)"), "should contain elapsed seconds");
    }

    #[test]
    fn test_tfn_no_args() {
        let result = tfn("tab.skills", "en", &[]);
        assert_eq!(result, "Skills", "no args should return unchanged");
    }

    #[test]
    fn console_copy_keys_have_no_banned_zh_jargon() {
        let banned = ["回执", "事项", "幕僚长", "主人", "边界", "拦截", "投影"];
        let mut hits = Vec::new();
        for (key, locales) in TRANSLATIONS.iter() {
            if !key.starts_with("console.") {
                continue;
            }
            let Some(zh) = locales.get("zh") else {
                continue;
            };
            for word in banned {
                if zh.contains(word) {
                    hits.push(format!("{key} contains banned {word}: {zh}"));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "banned jargon in console copy:\n{}",
            hits.join("\n")
        );
    }

    #[test]
    fn console_projection_sources_have_no_raw_english_sentences() {
        // Deliberate exceptions: HTTP/API error payloads, not Work/Home/Status copy.
        const ALLOW: &[(&str, &str)] = &[
            (
                "Invalid Web Push endpoint.",
                "HTTP validation error for a subscribe payload, not console copy",
            ),
            (
                "Invalid Web Push target URL.",
                "HTTP validation error for a notice URL, not console copy",
            ),
            (
                "Web Push send key is not configured.",
                "HTTP service error when VAPID private key is missing",
            ),
            (
                "Invalid Web Push {label} key.",
                "HTTP validation error for a VAPID key field, not console copy",
            ),
        ];
        let sources = [
            (
                "inbox_projection.rs",
                include_str!("admin/inbox_projection.rs"),
            ),
            (
                "vnext_projections.rs",
                include_str!("admin/vnext_projections.rs"),
            ),
            (
                "console_handlers.rs",
                include_str!("admin/console_handlers.rs"),
            ),
            ("daily_brief.rs", include_str!("daily_brief.rs")),
        ];
        let mut leftovers = Vec::new();
        for (name, src) in sources {
            for (line_no, literal) in owner_facing_english_sentence_literals(src) {
                if ALLOW.iter().any(|(allowed, _)| *allowed == literal) {
                    continue;
                }
                leftovers.push(format!("{name}:{line_no}: {literal}"));
            }
        }
        assert!(
            leftovers.is_empty(),
            "raw English owner sentences remain in console projection sources:\n{}",
            leftovers.join("\n")
        );
    }

    fn owner_facing_english_sentence_literals(src: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let mut in_block_comment = false;
        let mut in_test_mod = false;
        let mut test_mod_depth = 0i32;
        let mut brace_depth = 0i32;
        let mut pending_test_mod = false;
        for (idx, raw_line) in src.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();
            if pending_test_mod {
                if let Some(rest) = trimmed.strip_prefix("mod ") {
                    pending_test_mod = false;
                    in_test_mod = true;
                    test_mod_depth = brace_depth;
                    if rest.contains('{') {
                        brace_depth += rest.chars().filter(|c| *c == '{').count() as i32;
                        brace_depth -= rest.chars().filter(|c| *c == '}').count() as i32;
                    }
                    continue;
                }
                if trimmed.starts_with("pub(") || trimmed.starts_with("fn ") {
                    pending_test_mod = false;
                }
            }
            if trimmed == "#[cfg(test)]" {
                pending_test_mod = true;
            }
            if in_block_comment {
                if trimmed.contains("*/") {
                    in_block_comment = false;
                }
                continue;
            }
            if trimmed.starts_with("/*") {
                if !trimmed.contains("*/") {
                    in_block_comment = true;
                }
                continue;
            }
            if trimmed.starts_with("//") {
                continue;
            }
            let tracing_line = trimmed.contains("tracing::");
            brace_depth += raw_line.chars().filter(|c| *c == '{').count() as i32;
            brace_depth -= raw_line.chars().filter(|c| *c == '}').count() as i32;
            if in_test_mod && brace_depth <= test_mod_depth && trimmed.contains('}') {
                in_test_mod = false;
            }
            if in_test_mod || tracing_line {
                continue;
            }
            for literal in string_literals_in_line(raw_line) {
                if looks_like_english_sentence(&literal) {
                    out.push((line_no, literal));
                }
            }
        }
        out
    }

    fn string_literals_in_line(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] == '"' {
                i += 1;
                let mut buf = String::new();
                while i < chars.len() {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        buf.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        break;
                    }
                    buf.push(chars[i]);
                    i += 1;
                }
                out.push(buf);
            }
            i += 1;
        }
        out
    }

    fn looks_like_english_sentence(literal: &str) -> bool {
        let trimmed = literal.trim();
        if !trimmed.ends_with('.') {
            return false;
        }
        let words: Vec<&str> = trimmed
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == ':')
            .filter(|w| !w.is_empty())
            .collect();
        if words.len() < 4 {
            return false;
        }
        words.iter().all(|word| {
            word.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        '\'' | '-' | '(' | ')' | '/' | '.' | '!' | '?' | '{' | '}' | '0'..='9'
                    )
            })
        }) && words
            .iter()
            .any(|word| word.chars().any(|c| c.is_ascii_alphabetic()))
    }
}

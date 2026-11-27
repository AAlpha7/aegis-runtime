use serde::{Deserialize, Serialize};

use crate::i18n::{t, tfp};

const MAX_SUGGESTED_ACTIONS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledBriefKind {
    DailyBrief,
    EveningReview,
}

impl ScheduledBriefKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DailyBrief => "daily_brief",
            Self::EveningReview => "evening_review",
        }
    }

    pub const fn enabled_setting_key(self) -> &'static str {
        match self {
            Self::DailyBrief => "daily_brief_schedule_enabled",
            Self::EveningReview => "evening_review_schedule_enabled",
        }
    }

    pub const fn time_setting_key(self) -> &'static str {
        match self {
            Self::DailyBrief => "daily_brief_local_time",
            Self::EveningReview => "evening_review_local_time",
        }
    }

    pub const fn state_key(self) -> &'static str {
        match self {
            Self::DailyBrief => "last_daily_brief_local_date",
            Self::EveningReview => "last_evening_review_local_date",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledBriefConfig {
    pub enabled: bool,
    pub local_time: String,
}

impl ScheduledBriefConfig {
    pub fn disabled(default_time: impl Into<String>) -> Self {
        Self {
            enabled: false,
            local_time: default_time.into(),
        }
    }
}

pub fn parse_scheduled_brief_time(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    if !hour.chars().all(|ch| ch.is_ascii_digit()) || !minute.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let hour = hour.parse::<u32>().ok()?;
    let minute = minute.parse::<u32>().ok()?;
    if hour >= 24 || minute >= 60 {
        return None;
    }
    Some((hour, minute))
}

pub fn scheduled_brief_is_due(
    config: &ScheduledBriefConfig,
    local_date: &str,
    local_hour: u32,
    local_minute: u32,
    last_sent_local_date: Option<&str>,
) -> bool {
    if !config.enabled || last_sent_local_date == Some(local_date) {
        return false;
    }
    let Some((scheduled_hour, scheduled_minute)) = parse_scheduled_brief_time(&config.local_time)
    else {
        return false;
    };
    scheduled_brief_within_window(local_hour, local_minute, scheduled_hour, scheduled_minute)
}

/// Minutes after the scheduled time during which the brief may still be sent.
/// The scheduler ticks once a minute; an exact-minute match meant one slow
/// tick or a transient ledger error silently lost the whole day's brief.
pub const SCHEDULED_BRIEF_WINDOW_MINUTES: u32 = 180;

pub fn scheduled_brief_within_window(
    local_hour: u32,
    local_minute: u32,
    scheduled_hour: u32,
    scheduled_minute: u32,
) -> bool {
    let now = local_hour * 60 + local_minute;
    let scheduled = scheduled_hour * 60 + scheduled_minute;
    now >= scheduled && now - scheduled < SCHEDULED_BRIEF_WINDOW_MINUTES
}

pub fn scheduled_brief_is_due_with_loaded_state(
    config: &ScheduledBriefConfig,
    local_date: &str,
    local_hour: u32,
    local_minute: u32,
    state_available: bool,
    last_sent_local_date: Option<&str>,
) -> bool {
    if !state_available {
        return false;
    }
    scheduled_brief_is_due(
        config,
        local_date,
        local_hour,
        local_minute,
        last_sent_local_date,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BriefSourceStatus {
    Queried,
    NotQueried,
    Unavailable,
    AuthRequired,
    Error,
}

impl BriefSourceStatus {
    const fn label(self) -> &'static str {
        match self {
            Self::Queried => "queried",
            Self::NotQueried => "not_queried",
            Self::Unavailable => "unavailable",
            Self::AuthRequired => "auth_required",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefSourceState {
    pub status: BriefSourceStatus,
    pub detail: Option<String>,
}

impl BriefSourceState {
    pub fn queried() -> Self {
        Self {
            status: BriefSourceStatus::Queried,
            detail: None,
        }
    }

    pub fn not_queried(detail: impl Into<String>) -> Self {
        Self {
            status: BriefSourceStatus::NotQueried,
            detail: Some(detail.into()),
        }
    }

    pub fn auth_required(detail: impl Into<String>) -> Self {
        Self {
            status: BriefSourceStatus::AuthRequired,
            detail: Some(detail.into()),
        }
    }

    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            status: BriefSourceStatus::Unavailable,
            detail: Some(detail.into()),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            status: BriefSourceStatus::Error,
            detail: Some(detail.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefWarning {
    pub code: String,
    pub message: String,
}

impl BriefWarning {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarBriefState {
    pub source: BriefSourceState,
    pub events: Vec<CalendarBriefEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarBriefEvent {
    pub title: String,
    pub time: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBriefItem {
    pub title: String,
    pub due: Option<String>,
    pub priority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitingForItem {
    pub title: String,
    pub waiting_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalBriefItem {
    pub title: String,
    pub requested_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertBriefItem {
    pub title: String,
    pub severity: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BriefActionStatus {
    Proposed,
    PendingApproval,
    Executing,
    Executed,
    Failed,
    Skipped,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionLedgerSummary {
    pub summary: String,
    pub status: BriefActionStatus,
    pub receipt: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyBriefInput {
    pub local_date: String,
    pub timezone: String,
    pub generated_at: String,
    pub calendar: CalendarBriefState,
    pub tasks: Vec<TaskBriefItem>,
    pub waiting_for: Vec<WaitingForItem>,
    pub approvals: Vec<ApprovalBriefItem>,
    pub alerts: Vec<AlertBriefItem>,
    pub actions: Vec<ActionLedgerSummary>,
    pub suggested_actions: Vec<String>,
    pub warnings: Vec<BriefWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EveningReviewInput {
    pub local_date: String,
    pub timezone: String,
    pub generated_at: String,
    pub actions: Vec<ActionLedgerSummary>,
    pub pending_items: Vec<String>,
    pub durable_facts: Vec<String>,
    pub approvals: Vec<ApprovalBriefItem>,
    pub warnings: Vec<BriefWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefOutput {
    pub title: String,
    pub sections: Vec<BriefSection>,
    pub warnings: Vec<BriefWarning>,
    pub generated_at: String,
}

impl BriefOutput {
    pub fn section(&self, title: &str) -> Option<&BriefSection> {
        self.sections.iter().find(|section| section.title == title)
    }

    pub fn has_owner_visible_signal(&self) -> bool {
        self.warnings
            .iter()
            .any(|warning| brief_warning_needs_owner_attention(warning))
            || self
                .sections
                .iter()
                .any(|section| section.has_owner_visible_signal())
    }

    pub fn contains_line_fragment(&self, fragment: &str) -> bool {
        self.sections
            .iter()
            .flat_map(|section| section.lines.iter())
            .any(|line| line.contains(fragment))
    }

    pub fn to_markdown(&self, locale: &str) -> String {
        let mut out = String::new();
        out.push_str("# ");
        out.push_str(&self.title);
        out.push('\n');
        out.push('_');
        out.push_str(t("console.brief.generated", locale));
        out.push_str(&self.generated_at);
        out.push_str("_\n");

        for section in &self.sections {
            out.push_str("\n## ");
            out.push_str(&section.title);
            out.push('\n');
            for line in &section.lines {
                out.push_str("- ");
                out.push_str(line);
                out.push('\n');
            }
        }

        if !self.warnings.is_empty() {
            out.push_str("\n## ");
            out.push_str(t("console.brief.warnings", locale));
            out.push('\n');
            for warning in &self.warnings {
                out.push_str("- ");
                out.push_str(&warning.code);
                out.push_str(": ");
                out.push_str(&warning.message);
                out.push('\n');
            }
        }

        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefSection {
    pub title: String,
    pub lines: Vec<String>,
}

impl BriefSection {
    fn new(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            lines,
        }
    }

    fn has_owner_visible_signal(&self) -> bool {
        self.lines
            .iter()
            .any(|line| brief_line_has_owner_visible_signal(line))
    }
}

fn brief_warning_needs_owner_attention(warning: &BriefWarning) -> bool {
    !warning.code.ends_with("_not_queried")
}

fn brief_line_has_owner_visible_signal(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    let quiet_prefixes = [
        t("console.brief.calendar.not_queried", "en"),
        t("console.brief.calendar.not_queried", "zh"),
        t("console.brief.calendar.none", "en"),
        t("console.brief.calendar.none", "zh"),
        t("console.brief.tasks.empty", "en"),
        t("console.brief.tasks.empty", "zh"),
        t("console.brief.waiting.empty", "en"),
        t("console.brief.waiting.empty", "zh"),
        t("console.brief.approvals.empty", "en"),
        t("console.brief.approvals.empty", "zh"),
        t("console.brief.alerts.empty", "en"),
        t("console.brief.alerts.empty", "zh"),
        t("console.brief.actions.empty", "en"),
        t("console.brief.actions.empty", "zh"),
        t("console.brief.suggested.empty", "en"),
        t("console.brief.suggested.empty", "zh"),
        t("console.brief.pending.empty", "en"),
        t("console.brief.pending.empty", "zh"),
        t("console.brief.facts.empty", "en"),
        t("console.brief.facts.empty", "zh"),
    ];

    !quiet_prefixes
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

pub fn build_daily_brief(input: DailyBriefInput, locale: &str) -> BriefOutput {
    let mut warnings = input.warnings;
    let calendar = BriefSection::new(
        t("console.brief.section.calendar", locale),
        calendar_lines(&input.calendar, &mut warnings, locale),
    );
    let sections = vec![
        calendar,
        BriefSection::new(
            t("console.brief.section.tasks", locale),
            task_lines(&input.tasks, t("console.brief.tasks.empty", locale), locale),
        ),
        BriefSection::new(
            t("console.brief.section.waiting", locale),
            waiting_for_lines(&input.waiting_for, locale),
        ),
        BriefSection::new(
            t("console.brief.section.approvals", locale),
            approval_lines(&input.approvals, locale),
        ),
        BriefSection::new(
            t("console.brief.section.alerts", locale),
            alert_lines(&input.alerts, locale),
        ),
        BriefSection::new(
            t("console.brief.section.actions", locale),
            action_lines(&input.actions, locale),
        ),
        BriefSection::new(
            t("console.brief.section.suggested", locale),
            suggested_action_lines(&input.suggested_actions, locale),
        ),
    ];

    BriefOutput {
        title: tfp(
            "console.brief.daily.title",
            locale,
            &[&input.local_date, &input.timezone],
        ),
        sections,
        warnings,
        generated_at: input.generated_at,
    }
}

pub fn build_evening_review(input: EveningReviewInput, locale: &str) -> BriefOutput {
    let sections = vec![
        BriefSection::new(
            t("console.brief.section.evening_actions", locale),
            action_lines(&input.actions, locale),
        ),
        BriefSection::new(
            t("console.brief.section.pending", locale),
            pending_lines(&input.pending_items, locale),
        ),
        BriefSection::new(
            t("console.brief.section.approvals", locale),
            approval_lines(&input.approvals, locale),
        ),
        BriefSection::new(
            t("console.brief.section.facts", locale),
            durable_fact_lines(&input.durable_facts, locale),
        ),
    ];

    BriefOutput {
        title: tfp(
            "console.brief.evening.title",
            locale,
            &[&input.local_date, &input.timezone],
        ),
        sections,
        warnings: input.warnings,
        generated_at: input.generated_at,
    }
}

pub fn empty_daily_brief_input(
    local_date: impl Into<String>,
    timezone: impl Into<String>,
    generated_at: impl Into<String>,
) -> DailyBriefInput {
    DailyBriefInput {
        local_date: local_date.into(),
        timezone: timezone.into(),
        generated_at: generated_at.into(),
        calendar: CalendarBriefState {
            source: BriefSourceState::not_queried("manual brief input did not include calendar"),
            events: Vec::new(),
        },
        tasks: Vec::new(),
        waiting_for: Vec::new(),
        approvals: Vec::new(),
        alerts: Vec::new(),
        actions: Vec::new(),
        suggested_actions: Vec::new(),
        warnings: Vec::new(),
    }
}

pub fn empty_evening_review_input(
    local_date: impl Into<String>,
    timezone: impl Into<String>,
    generated_at: impl Into<String>,
) -> EveningReviewInput {
    EveningReviewInput {
        local_date: local_date.into(),
        timezone: timezone.into(),
        generated_at: generated_at.into(),
        actions: Vec::new(),
        pending_items: Vec::new(),
        durable_facts: Vec::new(),
        approvals: Vec::new(),
        warnings: Vec::new(),
    }
}

fn calendar_lines(
    calendar: &CalendarBriefState,
    warnings: &mut Vec<BriefWarning>,
    locale: &str,
) -> Vec<String> {
    match calendar.source.status {
        BriefSourceStatus::Queried => {
            if calendar.events.is_empty() {
                vec![t("console.brief.calendar.none", locale).to_string()]
            } else {
                calendar
                    .events
                    .iter()
                    .map(|event| {
                        let mut line = event.title.clone();
                        if let Some(time) = &event.time {
                            line.push_str(&tfp("console.brief.calendar.at", locale, &[time]));
                        }
                        if let Some(location) = &event.location {
                            line.push_str(&tfp(
                                "console.brief.calendar.location",
                                locale,
                                &[location],
                            ));
                        }
                        line
                    })
                    .collect()
            }
        }
        BriefSourceStatus::NotQueried => source_missing_lines(
            "calendar",
            &calendar.source,
            t("console.brief.calendar.not_queried", locale),
            warnings,
            locale,
        ),
        BriefSourceStatus::Unavailable
        | BriefSourceStatus::AuthRequired
        | BriefSourceStatus::Error => source_missing_lines(
            "calendar",
            &calendar.source,
            t("console.brief.calendar.unavailable", locale),
            warnings,
            locale,
        ),
    }
}

fn source_missing_lines(
    source: &str,
    state: &BriefSourceState,
    message: &str,
    warnings: &mut Vec<BriefWarning>,
    locale: &str,
) -> Vec<String> {
    let detail = state
        .detail
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(state.status.label());
    let warning = BriefWarning::new(format!("{source}_{}", state.status.label()), message);
    warnings.push(warning);
    vec![tfp("console.brief.detail", locale, &[message, detail])]
}

fn task_lines(tasks: &[TaskBriefItem], empty: &str, locale: &str) -> Vec<String> {
    if tasks.is_empty() {
        return vec![empty.to_string()];
    }

    tasks
        .iter()
        .map(|task| {
            let mut line = task.title.clone();
            if let Some(due) = &task.due {
                line.push_str(&tfp("console.brief.tasks.due", locale, &[due]));
            }
            if let Some(priority) = &task.priority {
                line.push_str(&format!(" [{priority}]"));
            }
            line
        })
        .collect()
}

fn waiting_for_lines(items: &[WaitingForItem], locale: &str) -> Vec<String> {
    if items.is_empty() {
        return vec![t("console.brief.waiting.empty", locale).to_string()];
    }

    items
        .iter()
        .map(|item| match &item.waiting_on {
            Some(who) => tfp("console.brief.waiting.on", locale, &[&item.title, who]),
            None => item.title.clone(),
        })
        .collect()
}

fn approval_lines(approvals: &[ApprovalBriefItem], locale: &str) -> Vec<String> {
    if approvals.is_empty() {
        return vec![t("console.brief.approvals.empty", locale).to_string()];
    }

    approvals
        .iter()
        .map(|approval| match &approval.requested_by {
            Some(requested_by) => tfp(
                "console.brief.approvals.by",
                locale,
                &[&approval.title, requested_by],
            ),
            None => approval.title.clone(),
        })
        .collect()
}

fn alert_lines(alerts: &[AlertBriefItem], locale: &str) -> Vec<String> {
    if alerts.is_empty() {
        return vec![t("console.brief.alerts.empty", locale).to_string()];
    }

    alerts
        .iter()
        .map(|alert| match &alert.severity {
            Some(severity) => tfp(
                "console.brief.alerts.severity",
                locale,
                &[&alert.title, severity],
            ),
            None => alert.title.clone(),
        })
        .collect()
}

fn brief_action_status_label(status: BriefActionStatus, locale: &str) -> &'static str {
    match status {
        BriefActionStatus::Proposed => t("console.brief.action.proposed", locale),
        BriefActionStatus::PendingApproval => t("console.brief.action.pending", locale),
        BriefActionStatus::Executing => t("console.brief.action.executing", locale),
        BriefActionStatus::Executed => t("console.brief.action.executed", locale),
        BriefActionStatus::Failed => t("console.brief.action.failed", locale),
        BriefActionStatus::Skipped => t("console.brief.action.skipped", locale),
        BriefActionStatus::Cancelled => t("console.brief.action.cancelled", locale),
        BriefActionStatus::Unknown => t("console.brief.action.unknown", locale),
    }
}

fn action_lines(actions: &[ActionLedgerSummary], locale: &str) -> Vec<String> {
    if actions.is_empty() {
        return vec![t("console.brief.actions.empty", locale).to_string()];
    }

    actions
        .iter()
        .map(|action| {
            let mut line = tfp(
                "console.brief.actions.line",
                locale,
                &[
                    brief_action_status_label(action.status, locale),
                    &action.summary,
                ],
            );
            if action.status == BriefActionStatus::Executed {
                match &action.receipt {
                    Some(receipt) => {
                        line.push_str(&tfp("console.brief.actions.receipt", locale, &[receipt]))
                    }
                    None => line.push_str(t("console.brief.actions.receipt_missing", locale)),
                }
            }
            if let Some(error) = &action.error {
                line.push_str(&tfp("console.brief.actions.error", locale, &[error]));
            }
            line
        })
        .collect()
}

fn suggested_action_lines(actions: &[String], locale: &str) -> Vec<String> {
    if actions.is_empty() {
        return vec![t("console.brief.suggested.empty", locale).to_string()];
    }

    actions
        .iter()
        .take(MAX_SUGGESTED_ACTIONS)
        .cloned()
        .collect()
}

fn pending_lines(items: &[String], locale: &str) -> Vec<String> {
    if items.is_empty() {
        return vec![t("console.brief.pending.empty", locale).to_string()];
    }

    items.to_vec()
}

fn durable_fact_lines(facts: &[String], locale: &str) -> Vec<String> {
    if facts.is_empty() {
        return vec![t("console.brief.facts.empty", locale).to_string()];
    }

    facts.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_daily_input() -> DailyBriefInput {
        DailyBriefInput {
            local_date: "2026-04-26".to_string(),
            timezone: "America/Los_Angeles".to_string(),
            generated_at: "2026-04-26T08:00:00-07:00".to_string(),
            calendar: CalendarBriefState {
                source: BriefSourceState::queried(),
                events: Vec::new(),
            },
            tasks: Vec::new(),
            waiting_for: Vec::new(),
            approvals: Vec::new(),
            alerts: Vec::new(),
            actions: Vec::new(),
            suggested_actions: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn base_evening_input() -> EveningReviewInput {
        EveningReviewInput {
            local_date: "2026-04-26".to_string(),
            timezone: "America/Los_Angeles".to_string(),
            generated_at: "2026-04-26T20:00:00-07:00".to_string(),
            actions: Vec::new(),
            pending_items: Vec::new(),
            durable_facts: Vec::new(),
            approvals: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn daily_brief_calendar_not_queried_does_not_claim_no_events() {
        let mut input = base_daily_input();
        input.calendar = CalendarBriefState {
            source: BriefSourceState::not_queried("calendar permission not checked"),
            events: Vec::new(),
        };

        let output = build_daily_brief(input, "en");

        assert!(output.contains_line_fragment("Calendar not queried"));
        assert!(!output.contains_line_fragment("no events were provided"));
        assert!(
            output
                .warnings
                .iter()
                .any(|warning| warning.code == "calendar_not_queried")
        );
    }

    #[test]
    fn empty_daily_brief_has_no_owner_visible_signal() {
        let output = build_daily_brief(
            empty_daily_brief_input(
                "2026-05-22",
                "America/Los_Angeles",
                "2026-05-22T09:00:00-07:00",
            ),
            "en",
        );

        assert!(!output.has_owner_visible_signal());
        assert!(output.contains_line_fragment("Calendar not queried"));
    }

    #[test]
    fn daily_brief_with_real_work_has_owner_visible_signal() {
        let mut input = empty_daily_brief_input(
            "2026-05-22",
            "America/Los_Angeles",
            "2026-05-22T09:00:00-07:00",
        );
        input.approvals.push(ApprovalBriefItem {
            title: "Approve calendar event create".to_string(),
            requested_by: Some("calendar_reader".to_string()),
        });

        let output = build_daily_brief(input, "en");

        assert!(output.has_owner_visible_signal());
    }

    #[test]
    fn unavailable_source_warning_has_owner_visible_signal() {
        let mut input = base_daily_input();
        input.calendar = CalendarBriefState {
            source: BriefSourceState::auth_required("owner needs to reconnect Calendar"),
            events: Vec::new(),
        };

        let output = build_daily_brief(input, "en");

        assert!(output.has_owner_visible_signal());
    }

    #[test]
    fn daily_brief_includes_pending_approvals() {
        let mut input = base_daily_input();
        input.approvals.push(ApprovalBriefItem {
            title: "Approve corpus re-embed".to_string(),
            requested_by: Some("health probe".to_string()),
        });

        let output = build_daily_brief(input, "en");

        let approvals = output.section("Pending Approvals").unwrap();
        assert_eq!(
            approvals.lines,
            vec!["Approve corpus re-embed -- requested by health probe"]
        );
    }

    #[test]
    fn daily_brief_limits_top_suggested_actions() {
        let mut input = base_daily_input();
        input.suggested_actions = vec![
            "Retry lawyer wire".to_string(),
            "Call bank if wire fails".to_string(),
            "Review USPS notice".to_string(),
            "Archive old invoices".to_string(),
        ];

        let output = build_daily_brief(input, "en");

        let suggested = output.section("Suggested Actions").unwrap();
        assert_eq!(suggested.lines.len(), 3);
        assert!(
            !suggested
                .lines
                .iter()
                .any(|line| line == "Archive old invoices")
        );
    }

    #[test]
    fn evening_review_distinguishes_executed_from_proposed() {
        let mut input = base_evening_input();
        input.actions = vec![
            ActionLedgerSummary {
                summary: "Sent heartbeat alert".to_string(),
                status: BriefActionStatus::Executed,
                receipt: Some("telegram:1:2".to_string()),
                error: None,
            },
            ActionLedgerSummary {
                summary: "Draft bank follow-up".to_string(),
                status: BriefActionStatus::Proposed,
                receipt: None,
                error: None,
            },
        ];

        let output = build_evening_review(input, "en");

        assert!(output.contains_line_fragment("Executed: Sent heartbeat alert"));
        assert!(output.contains_line_fragment("receipt: telegram:1:2"));
        assert!(output.contains_line_fragment("Proposed: Draft bank follow-up"));
    }

    #[test]
    fn evening_review_includes_failed_actions() {
        let mut input = base_evening_input();
        input.actions.push(ActionLedgerSummary {
            summary: "Send Telegram alert".to_string(),
            status: BriefActionStatus::Failed,
            receipt: None,
            error: Some("telegram_send_failed".to_string()),
        });

        let output = build_evening_review(input, "en");

        assert!(output.contains_line_fragment("Failed: Send Telegram alert"));
        assert!(output.contains_line_fragment("telegram_send_failed"));
    }

    #[test]
    fn evening_review_does_not_hide_skipped_actions() {
        let mut input = base_evening_input();
        input.actions.push(ActionLedgerSummary {
            summary: "Skip low-priority digest".to_string(),
            status: BriefActionStatus::Skipped,
            receipt: None,
            error: None,
        });

        let output = build_evening_review(input, "en");

        assert!(output.contains_line_fragment("Skipped: Skip low-priority digest"));
    }

    #[test]
    fn brief_outputs_warning_for_auth_required_source() {
        let mut input = base_daily_input();
        input.calendar = CalendarBriefState {
            source: BriefSourceState::auth_required("Calendar OAuth expired"),
            events: Vec::new(),
        };

        let output = build_daily_brief(input, "en");

        assert!(output.contains_line_fragment("Calendar unavailable"));
        assert!(
            output
                .warnings
                .iter()
                .any(|warning| warning.code == "calendar_auth_required")
        );
    }

    #[test]
    fn brief_does_not_require_llm_provider() {
        let mut input = base_daily_input();
        input.tasks.push(TaskBriefItem {
            title: "Prepare claim documents".to_string(),
            due: Some("today".to_string()),
            priority: Some("high".to_string()),
        });

        let output = build_daily_brief(input, "en");

        assert_eq!(
            output.title,
            "Daily Brief for 2026-04-26 (America/Los_Angeles)"
        );
        assert!(output.contains_line_fragment("Prepare claim documents due today [high]"));
    }

    #[test]
    fn empty_daily_brief_input_is_honest_about_missing_sources() {
        let input = empty_daily_brief_input(
            "2026-04-26",
            "America/Los_Angeles",
            "2026-04-26T08:00:00-07:00",
        );

        let output = build_daily_brief(input, "en");

        assert!(output.contains_line_fragment("Calendar not queried"));
        assert!(!output.contains_line_fragment("no events were provided"));
    }

    #[test]
    fn brief_output_renders_markdown_without_llm() {
        let mut input = base_evening_input();
        input
            .pending_items
            .push("Review tomorrow's calendar".to_string());

        let output = build_evening_review(input, "en");
        let markdown = output.to_markdown("en");

        assert!(markdown.contains("# Evening Review for 2026-04-26"));
        assert!(markdown.contains("## Still Pending"));
        assert!(markdown.contains("- Review tomorrow's calendar"));
    }

    #[test]
    fn scheduled_brief_time_parser_accepts_strict_hhmm() {
        assert_eq!(parse_scheduled_brief_time("00:00"), Some((0, 0)));
        assert_eq!(parse_scheduled_brief_time("08:30"), Some((8, 30)));
        assert_eq!(parse_scheduled_brief_time("23:59"), Some((23, 59)));
    }

    #[test]
    fn scheduled_brief_time_parser_rejects_invalid_values() {
        for value in ["8:30", "08:3", "24:00", "23:60", "0830", "aa:bb", " 08:30"] {
            assert_eq!(parse_scheduled_brief_time(value), None, "{value}");
        }
    }

    #[test]
    fn scheduled_brief_due_requires_enabled_exact_minute_and_unsent_date() {
        let config = ScheduledBriefConfig {
            enabled: true,
            local_time: "08:30".to_string(),
        };

        assert!(scheduled_brief_is_due(&config, "2026-04-27", 8, 30, None));
        assert!(!scheduled_brief_is_due(&config, "2026-04-27", 8, 29, None));
        assert!(!scheduled_brief_is_due(
            &config,
            "2026-04-27",
            8,
            30,
            Some("2026-04-27")
        ));
        assert!(scheduled_brief_is_due(
            &config,
            "2026-04-27",
            8,
            30,
            Some("2026-04-26")
        ));
    }

    #[test]
    fn scheduled_brief_due_respects_disabled_config() {
        let config = ScheduledBriefConfig::disabled("08:30");

        assert!(!scheduled_brief_is_due(&config, "2026-04-27", 8, 30, None));
    }

    #[test]
    fn scheduled_brief_loaded_state_deduplicates_by_local_date_after_utc_rollover() {
        let config = ScheduledBriefConfig {
            enabled: true,
            local_time: "00:30".to_string(),
        };

        assert!(!scheduled_brief_is_due_with_loaded_state(
            &config,
            "2026-05-01",
            0,
            30,
            true,
            Some("2026-05-01")
        ));
        assert!(scheduled_brief_is_due_with_loaded_state(
            &config,
            "2026-05-01",
            0,
            30,
            true,
            Some("2026-04-30")
        ));
    }

    #[test]
    fn scheduled_brief_loaded_state_skips_when_state_unavailable() {
        let config = ScheduledBriefConfig {
            enabled: true,
            local_time: "08:30".to_string(),
        };

        assert!(!scheduled_brief_is_due_with_loaded_state(
            &config,
            "2026-04-27",
            8,
            30,
            false,
            None
        ));
    }

    #[test]
    fn scheduled_brief_loaded_state_rejects_malformed_time() {
        let config = ScheduledBriefConfig {
            enabled: true,
            local_time: "8:30".to_string(),
        };

        assert!(!scheduled_brief_is_due_with_loaded_state(
            &config,
            "2026-04-27",
            8,
            30,
            true,
            None
        ));
    }

    #[test]
    fn scheduled_brief_due_window_survives_a_missed_minute_but_not_the_afternoon() {
        let config = ScheduledBriefConfig {
            enabled: true,
            local_time: "08:30".to_string(),
        };
        assert!(
            scheduled_brief_is_due(&config, "2026-04-27", 8, 31, None),
            "one minute late still sends"
        );
        assert!(
            scheduled_brief_is_due(&config, "2026-04-27", 10, 0, None),
            "90 minutes late still sends"
        );
        assert!(
            !scheduled_brief_is_due(&config, "2026-04-27", 11, 30, None),
            "window closes after 3h"
        );
        assert!(!scheduled_brief_is_due(&config, "2026-04-27", 8, 29, None));
        assert!(
            !scheduled_brief_is_due(&config, "2026-04-27", 9, 0, Some("2026-04-27")),
            "already sent today"
        );
        assert!(scheduled_brief_within_window(9, 0, 8, 30));
        assert!(!scheduled_brief_within_window(11, 31, 8, 30));
    }

    #[test]
    fn zh_daily_brief_has_no_ascii_only_sentence_or_banned_jargon() {
        let output = build_daily_brief(base_daily_input(), "zh");
        let blob = format!(
            "{}\n{}",
            output.title,
            output
                .sections
                .iter()
                .flat_map(|section| std::iter::once(section.title.as_str())
                    .chain(section.lines.iter().map(String::as_str)))
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert_no_ascii_only_sentence_or_banned_zh(&blob);
    }

    fn assert_no_ascii_only_sentence_or_banned_zh(blob: &str) {
        for banned in ["回执", "事项", "幕僚长", "主人", "边界", "拦截", "投影"] {
            assert!(
                !blob.contains(banned),
                "zh copy contains banned jargon {banned}: {blob}"
            );
        }
        for sentence in blob.split(['。', '.', '！', '!', '？', '?']) {
            let trimmed = sentence.trim();
            if trimmed.is_empty() {
                continue;
            }
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            if words.len() >= 4 && trimmed.is_ascii() {
                panic!("zh copy still has an ASCII-only sentence: {trimmed}");
            }
        }
    }
}

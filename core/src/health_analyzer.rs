//! Periodic health analyzer for skill metrics.
//!
//! Scans skill_metrics periodically and generates health reports.
//! When a skill's error rate exceeds a threshold, it recommends repair
//! and notifies the user via a callback channel.
//!
//! Design principle: recommend only, never auto-act. Respects user agency.
//!
//! # Integration status: ACTIVE
//!
//! Wired in `main.rs` startup (section 10c-iii). Sends health
//! recommendations via mpsc channel to the Telegram notification task.

use crate::memory::MemoryHandle;
use tokio::sync::mpsc;

/// Threshold: if error_rate > this over the observation window, recommend repair.
const ERROR_RATE_THRESHOLD: f64 = 0.50;

/// Minimum number of calls before health analysis is meaningful.
const MIN_CALLS_FOR_ANALYSIS: u64 = 10;

/// Interval between health scans (1 hour).
const SCAN_INTERVAL_SECS: u64 = 3600;

/// A health recommendation for a degraded skill.
#[derive(Debug, Clone)]
pub struct HealthRecommendation {
    pub skill_name: String,
    pub error_rate: f64,
    pub total_calls: u64,
    pub error_count: u64,
    pub last_error: Option<String>,
    pub recommendation: String,
}

/// Analyze all skill metrics and return recommendations for degraded skills.
pub async fn analyze_skill_health(memory: &MemoryHandle) -> Vec<HealthRecommendation> {
    let metrics = match memory.get_all_skill_metrics().await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get skill metrics for health analysis");
            return Vec::new();
        }
    };

    let mut recommendations = Vec::new();

    for m in &metrics {
        if m.total_calls < MIN_CALLS_FOR_ANALYSIS {
            continue;
        }

        let error_rate = m.error_count as f64 / m.total_calls as f64;

        if error_rate > ERROR_RATE_THRESHOLD {
            let recommendation = format!(
                "Skill '{}' has a {:.0}% error rate ({}/{} calls failed). \
                 Last error: {}. Consider deactivate_skill / reactivate_skill to reset it.",
                m.skill_name,
                error_rate * 100.0,
                m.error_count,
                m.total_calls,
                m.last_error.as_deref().unwrap_or("(unknown)"),
            );

            recommendations.push(HealthRecommendation {
                skill_name: m.skill_name.clone(),
                error_rate,
                total_calls: m.total_calls,
                error_count: m.error_count,
                last_error: m.last_error.clone(),
                recommendation,
            });
        }
    }

    // Sort by error rate (worst first)
    recommendations.sort_by(|a, b| {
        b.error_rate
            .partial_cmp(&a.error_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    recommendations
}

/// Spawn a background health analyzer that periodically scans skill metrics
/// and sends recommendations via the provided channel.
///
/// Returns a `JoinHandle` that can be used to abort the task.
pub fn spawn_health_analyzer(
    memory: MemoryHandle,
    notify_tx: mpsc::Sender<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(SCAN_INTERVAL_SECS));

        // Skip the first tick (fires immediately)
        interval.tick().await;
        let mut gate = crate::alert_gate::AlertGate::new(HEALTH_RENAG, HEALTH_RENAG);

        loop {
            interval.tick().await;

            let recommendations = analyze_skill_health(&memory).await;
            let to_notify =
                select_health_alerts(&recommendations, &mut gate, std::time::Instant::now());

            if recommendations.is_empty() {
                tracing::debug!("Health analyzer: all skills healthy");
                continue;
            }

            tracing::info!(
                degraded_count = recommendations.len(),
                notify_count = to_notify.len(),
                "Health analyzer found degraded skills"
            );

            for rec in to_notify {
                let msg = format!(
                    "[Health Monitor] ⚠ {}\n\
                     Error rate: {:.0}% ({}/{} calls)\n\
                     Last error: {}\n\
                     Recommendation: Use deactivate_skill / reactivate_skill to reset it.",
                    rec.skill_name,
                    rec.error_rate * 100.0,
                    rec.error_count,
                    rec.total_calls,
                    rec.last_error.as_deref().unwrap_or("unknown"),
                );
                if notify_tx.try_send(msg).is_err() {
                    tracing::debug!("Health notification channel full — skipping");
                }
            }
        }
    })
}

/// Re-nag interval for a skill that stays degraded (metrics are cumulative
/// and never decay, so without this the same skill alerts every scan forever).
const HEALTH_RENAG: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Pick which recommendations to notify about: a skill alerts when it first
/// shows up degraded, is silenced while it stays degraded (re-nag daily), and
/// alerts again only after it recovered and relapsed.
pub fn select_health_alerts<'a>(
    recommendations: &'a [HealthRecommendation],
    gate: &mut crate::alert_gate::AlertGate,
    now: std::time::Instant,
) -> Vec<&'a HealthRecommendation> {
    let degraded: std::collections::HashSet<&str> = recommendations
        .iter()
        .map(|r| r.skill_name.as_str())
        .collect();
    for key in gate.failing_keys() {
        if !degraded.contains(key.as_str()) {
            gate.record_success(&key);
        }
    }
    recommendations
        .iter()
        .filter(|r| gate.record_failure(&r.skill_name, 1, now))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str) -> HealthRecommendation {
        HealthRecommendation {
            skill_name: name.to_string(),
            error_rate: 0.9,
            error_count: 9,
            total_calls: 10,
            last_error: Some("boom".to_string()),
            recommendation: "deactivate_skill".to_string(),
        }
    }

    #[test]
    fn degraded_skill_is_reported_once_then_silenced_until_renag() {
        let mut gate = crate::alert_gate::AlertGate::new(
            std::time::Duration::from_secs(86_400),
            std::time::Duration::from_secs(86_400),
        );
        let t0 = std::time::Instant::now();
        let recs = vec![rec("a"), rec("b")];
        assert_eq!(select_health_alerts(&recs, &mut gate, t0).len(), 2);
        assert!(
            select_health_alerts(&recs, &mut gate, t0 + std::time::Duration::from_secs(3600))
                .is_empty(),
            "same degraded set an hour later: no re-alert"
        );
        let only_a = vec![rec("a")];
        assert!(
            select_health_alerts(
                &only_a,
                &mut gate,
                t0 + std::time::Duration::from_secs(7200)
            )
            .is_empty()
        );
        let recs2 = vec![rec("a"), rec("b")];
        let again = select_health_alerts(
            &recs2,
            &mut gate,
            t0 + std::time::Duration::from_secs(10_000),
        );
        assert_eq!(
            again.len(),
            1,
            "b recovered then degraded again: alert only b"
        );
        assert_eq!(again[0].skill_name, "b");
    }

    #[tokio::test]
    async fn analyze_healthy_skills_returns_empty() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();
        // Record some successful calls
        mem.update_skill_metrics("test_skill", true, 100.0, None, None)
            .await;
        mem.update_skill_metrics("test_skill", true, 150.0, None, None)
            .await;
        mem.update_skill_metrics("test_skill", true, 120.0, None, None)
            .await;

        // Need at least MIN_CALLS_FOR_ANALYSIS calls
        for _ in 0..10 {
            mem.update_skill_metrics("test_skill", true, 100.0, None, None)
                .await;
        }

        // Give the actor time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recs = analyze_skill_health(&mem).await;
        assert!(
            recs.is_empty(),
            "healthy skills should produce no recommendations"
        );
    }

    #[tokio::test]
    async fn analyze_degraded_skill_recommends_repair() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        // Record mostly failures
        for _ in 0..8 {
            mem.update_skill_metrics("bad_skill", false, 500.0, Some("API timeout"), None)
                .await;
        }
        for _ in 0..2 {
            mem.update_skill_metrics("bad_skill", true, 100.0, None, None)
                .await;
        }

        // Give the actor time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recs = analyze_skill_health(&mem).await;
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].skill_name, "bad_skill");
        assert!(recs[0].error_rate > 0.5);
        assert!(recs[0].recommendation.contains("bad_skill"));
    }

    #[tokio::test]
    async fn analyze_skips_low_volume_skills() {
        let mem = MemoryHandle::spawn_in_memory().unwrap();

        // Only 3 calls — below MIN_CALLS_FOR_ANALYSIS
        for _ in 0..3 {
            mem.update_skill_metrics("new_skill", false, 500.0, Some("error"), None)
                .await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recs = analyze_skill_health(&mem).await;
        assert!(recs.is_empty(), "low-volume skills should be skipped");
    }
}

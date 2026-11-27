# Aegis Runtime Documentation Index

Date: 2026-09-15
Status: active docs entry point

## Start Here

- [V1 decisions and acceptance checklist](V1_ACCEPTANCE_CHECKLIST.md) — current requirements, fixes, pending decisions and real acceptance gaps
- [2026-09-28 verification](verification-2026-09-28.md) — DDG removal, Grok/connector acceptance and Windows recovery
- `README.md` (repo root) — quick start: system deps, build the console, build the binary, run, open the printed login link
- `docs/DEPLOYMENT.md` — 部署指南：环境、配置、生产部署、故障排查（Chinese）
- `docs/DESIGN.md` — 系统设计与工程实现规范（Chinese）
- `docs/TESTING_GUIDE.md` — test layers, commands, LLM-boundary checklist
- `docs/SKILL_DEVELOPMENT.md` — Wasm 技能开发指南（Chinese）
- `docs/agent/README.md` — repo-agent playbook: how to verify a change
- [Heirloom status and opt-in](HEIRLOOM.md) — experimental, disabled by default
- [Subscription execution and search](SUBSCRIPTIONS.md) — subscription routing, explicit search failure, connector limits and deferred realtime voice

## Product Direction

The WebApp (Web Console at `/ui/`) is the primary owner surface. Telegram is a
notification / deep-link fallback, and subscription providers are preferred for
foreground reasoning. Google Calendar / Drive / Gmail work is delegated to the
external executor (Codex) rather than run in-process.

Decision records behind the current shape:

- [手机访问安装步骤](REMOTE_ACCESS.md) — portable 部署选择、Passkey 配对、恢复和无需账号的测试
- [本机部署与手机访问模式](remote-access-modes.md) — 设计与验收边界：仅本机、私网 Serve 或公网 Funnel；公网需明确风险确认，不能绕过安全检查
- `docs/2026-09-04-feature-removal-plan.md` — what was removed on 2026-09-05
  (SkillForge, Dev Harness, learning / initiative / research, browser agent, agent swarm) and why
- 2026-09-07 (owner decision, record archived with the historical docs): the self-built mail client was removed; mail, calendar and Drive work is delegated to external executors
- [Optional Hercules delegation](../README.md#optional-hercules-delegation) — `delegate_task` (default: off; you run and configure Hercules yourself)

## Operational Runbooks And References

- `docs/deployment-prereqs.md` — fresh-machine host prerequisites
- `Docs/cursor-cloud-opensource-deploy-status.md` — Cursor Cloud verification of the public self-host path
- `docs/pre-deploy-audit-summary-template.md` — pre-deploy audit summary shape
- `docs/post-deploy-checklist-plan.md` — post-deploy checklist (used by `scripts/post-deploy-checklist.sh`)
- `docs/three-tier-test-strategy.md` — commit gate / deploy freshness / nightly full run
- `docs/multi-cc-canary-runbook.md` — coordination-layer canary smoke (author-local `.claude/` hooks)
- `docs/multi-cc-team-coordination-plan.md` — multi-agent coordination design referenced by `scripts/claude-*.sh`
- `docs/remote-steward.md` — `scripts/remote-steward.mjs` operations sensor
- `docs/codex-agent-bus-v1.md` — agent coordination protocol (only when asked to coordinate)
- `docs/codex-oauth-provider-integration-guide.md` — Codex app-server provider integration
- `docs/profile-registry.md` — profile slot registry (source of truth: `core/src/profile_registry.rs`)

## Active Design Set

WebApp / console product:

- `docs/design/2026-05-09-personal-secretary-ux-north-star.md`
- `docs/design/2026-05-09-webapp-vnext-decision-log.md`
- `docs/design/2026-05-09-webapp-vnext-product-architecture.md`
- `docs/design/2026-05-09-webapp-vnext-phase-0-implementation-spec.md`
- `docs/design/2026-05-15-webapp-v2-chat-home.md`
- `docs/2026-05-31-webapp-aurora-glass-redesign-design.md` — design system referenced from `webapp/src/vnext/design/`
- `docs/2026-06-09-webapp-transport-ux-design.md`
- `docs/design/2026-07-25-ceo-intent-contract.md` and `docs/design/2026-07-25-ceo-voice-assistant.md`
- `docs/design/webapp-ux-gallery/README.md` — visual north star
- `docs/superpowers/specs/2026-06-27-button-design-system-design.md`, `2026-06-27-webapp-chrome-controls-audit.md`, `2026-06-29-webapp-nav-model-redesign-design.md`
- `docs/superpowers/specs/2026-06-07-i18n-language-consistency-design.md` and its plan

Trust layer and security:

- `docs/design/2026-05-02-web-console-trust-layer-design-index.md`
- `docs/design/p0-sdui-web-console-security-hardening.md`
- `docs/design/p0-sensitive-read-policy-data-access-gate.md`
- `docs/design/p0-web-console-google-login-session-auth.md`
- `docs/design/p0-web-console-passkey-mfa-step-up.md`
- `docs/design/p1-notification-privacy-redaction.md`
- `docs/design/p1-action-ledger.md`

Runtime, providers, notifications, memory:

- `docs/design/p0-llm-provider-routing-subscription-first.md`
- `docs/design/p0-grok-subscription-provider-integration.md`
- `docs/design/p0-heartbeat-noise-cost-cleanup.md`, `docs/design/p2-heartbeat-rolling-window-monitor.md`, `docs/heartbeat-autonomy-design.md`
- `docs/design/p0-telegram-notification-router.md`, `docs/design/p2-notification-auth-warning-review.md`
- `docs/design/p1-daily-brief-evening-review.md`
- `docs/design/p1-kb-dedup-dashboard.md`
- `docs/design/p1-webapp-mandate-proactive-collaboration.md`
- `docs/design/p2-heirloom-projectors-read-migration.md`
- `docs/design/p2-production-log-forensic-audit.md`, `docs/design/p2-google-auth-audit-classifier.md`, `docs/design/p2-provider-local-cost-reconciliation.md`, `docs/design/p2-stale-work-triage.md`
- `docs/design/p2-ios-mobile-cockpit-boundaries.md`
- `docs/design/implementation-discipline.md` — implementation rules the design docs above refer to
- `docs/subcall-primitive-design.md`, `docs/subcall-primitive-implementation-plan.md` — subcall primitive (referenced from `core/src/subcall*.rs`)

Repo-agent harness: `docs/agent/*` (see `AGENTS.md`). Heirloom: `docs/heirloom/*`.

## Historical Records

Dated plans, audits, reference analyses and every document about a removed
feature (SkillForge, the self-built mail client and its mail proxy, browser agent,
initiative / research engine, Dev Harness) were moved out of the repository on
2026-09-15 into the maintainer's archive. Links to those paths inside the
remaining design docs are historical and intentionally not rewritten.

Some kept docs still mention a removed feature in passing; each carries a
one-line italic banner at the top saying so. Treat such mentions as history,
and verify current behavior against the code and the active design set.

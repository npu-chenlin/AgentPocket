use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::model::{AgentEvent, AgentEventKind, Backend, DesktopSettings, ServerConfig};

const DEFAULT_TTL_SECONDS: i64 = 30 * 60;
const DEFAULT_CAPACITY: usize = 1024;

/// A notification that has passed the deduplication policy and is ready to be shown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingNotification {
    pub title: String,
    pub body: String,
}

/// Pure, testable deduplication policy for agent events.
#[derive(Clone, Debug)]
pub struct NotificationPolicy {
    seen: HashMap<String, DateTime<Utc>>,
    ttl: Duration,
    capacity: usize,
}

impl NotificationPolicy {
    /// Create a policy with the given TTL and capacity.
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            seen: HashMap::new(),
            ttl,
            capacity,
        }
    }

    /// Default 30-minute deduplication window.
    pub fn default_ttl() -> Duration {
        Duration::from_secs(DEFAULT_TTL_SECONDS as u64)
    }

    /// Default maximum number of tracked event keys.
    pub fn default_capacity() -> usize {
        DEFAULT_CAPACITY
    }

    /// Decide whether an event should be emitted as a desktop notification.
    ///
    /// Baseline events (`occurred_at <= monitor_started_at`) are suppressed,
    /// as are duplicate keys within the TTL and event kinds that do not require
    /// user attention.
    pub fn should_send(
        &mut self,
        event: &AgentEvent,
        monitor_started_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> bool {
        if !is_notifiable_kind(&event.kind) {
            return false;
        }

        if event.occurred_at <= monitor_started_at {
            return false;
        }

        let key = dedup_key(event);
        let ttl = chrono_ttl(self.ttl);

        if let Some(&last_seen) = self.seen.get(&key) {
            if now.signed_duration_since(last_seen) < ttl {
                return false;
            }
        }

        self.evict_if_needed(now, ttl);
        self.seen.insert(key, now);
        true
    }

    fn evict_if_needed(&mut self, now: DateTime<Utc>, ttl: chrono::Duration) {
        // Drop expired entries first so real duplicates do not accumulate.
        self.seen
            .retain(|_, &mut last_seen| now.signed_duration_since(last_seen) < ttl);

        if self.seen.len() >= self.capacity {
            // Still at capacity: evict the oldest entries until there is room.
            let mut entries: Vec<(String, DateTime<Utc>)> = self.seen.drain().collect();
            entries.sort_by_key(|a| a.1);
            let remove_count = entries.len().saturating_sub(self.capacity - 1);
            entries.drain(..remove_count);
            self.seen.extend(entries);
        }
    }
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self::new(Self::default_ttl(), Self::default_capacity())
    }
}

/// Coordinates the policy check with the Tauri notification plugin.
pub struct NotificationCoordinator {
    policy: NotificationPolicy,
}

impl NotificationCoordinator {
    pub fn new() -> Self {
        Self {
            policy: NotificationPolicy::default(),
        }
    }

    /// Translate an agent event into a desktop notification if it passes the
    /// user settings and deduplication policy.
    pub fn handle_event(
        &mut self,
        event: &AgentEvent,
        server: &ServerConfig,
        monitor_started_at: DateTime<Utc>,
        now: DateTime<Utc>,
        settings: &DesktopSettings,
    ) -> Option<PendingNotification> {
        if !settings.notifications {
            return None;
        }

        if !self.policy.should_send(event, monitor_started_at, now) {
            return None;
        }

        Some(PendingNotification {
            title: notification_title(server.backend, &event.kind),
            body: event
                .body
                .clone()
                .or_else(|| event.session_title.clone())
                .unwrap_or_default(),
        })
    }

    /// Show a pending notification through the Tauri notification plugin.
    pub fn show<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        pending: PendingNotification,
    ) -> Result<(), tauri_plugin_notification::Error> {
        use tauri_plugin_notification::NotificationExt;

        app.notification()
            .builder()
            .title(pending.title)
            .body(pending.body)
            .show()
    }
}

impl Default for NotificationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

fn is_notifiable_kind(kind: &AgentEventKind) -> bool {
    matches!(
        kind,
        AgentEventKind::Completed
            | AgentEventKind::Failed
            | AgentEventKind::ApprovalRequired
            | AgentEventKind::QuestionRequired
    )
}

fn dedup_key(event: &AgentEvent) -> String {
    format!(
        "{}:{}:{}",
        event.server_id,
        event.session_id.as_deref().unwrap_or(""),
        event.event_key
    )
}

fn notification_title(backend: Backend, kind: &AgentEventKind) -> String {
    let prefix = match backend {
        Backend::Kimi => "Kimi Code",
        Backend::Dsh => "DeepSeek Harness",
    };
    let action = match kind {
        AgentEventKind::Completed => "任务完成",
        AgentEventKind::Failed => "任务失败",
        AgentEventKind::ApprovalRequired => "等待审批",
        AgentEventKind::QuestionRequired => "待回答",
    };
    format!("{} · {}", prefix, action)
}

fn chrono_ttl(ttl: Duration) -> chrono::Duration {
    chrono::Duration::from_std(ttl)
        .unwrap_or_else(|_| chrono::Duration::seconds(DEFAULT_TTL_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        server_id: &str,
        session_id: Option<&str>,
        event_key: &str,
        kind: AgentEventKind,
        occurred_at: DateTime<Utc>,
    ) -> AgentEvent {
        AgentEvent {
            server_id: server_id.to_string(),
            session_id: session_id.map(|s| s.to_string()),
            session_title: Some("test session".to_string()),
            kind,
            event_key: event_key.to_string(),
            body: Some("test body".to_string()),
            occurred_at,
        }
    }

    fn server(backend: Backend) -> ServerConfig {
        ServerConfig::new("s1", "Server", "100.64.0.2", 3080, "secret-token", backend)
    }

    #[test]
    fn duplicate_event_is_suppressed_for_thirty_minutes() {
        let mut policy = NotificationPolicy::default();
        let started = Utc::now();
        let event1 = event(
            "srv",
            Some("sess"),
            "k1",
            AgentEventKind::Completed,
            started + chrono::Duration::seconds(1),
        );
        let event2 = event(
            "srv",
            Some("sess"),
            "k1",
            AgentEventKind::Completed,
            started + chrono::Duration::seconds(2),
        );

        assert!(policy.should_send(&event1, started, started + chrono::Duration::seconds(1)));
        assert!(!policy.should_send(&event2, started, started + chrono::Duration::seconds(2)));

        // Just inside the TTL window it is still suppressed.
        let just_inside =
            started + chrono::Duration::seconds(1) + NotificationPolicy::default_ttl();
        assert!(!policy.should_send(&event2, started, just_inside - chrono::Duration::seconds(1)));

        // One second after the TTL expires it may be sent again.
        let outside = started
            + chrono::Duration::seconds(1)
            + NotificationPolicy::default_ttl()
            + chrono::Duration::seconds(1);
        assert!(policy.should_send(&event2, started, outside));
    }

    #[test]
    fn same_event_key_on_different_servers_is_not_duplicate() {
        let mut policy = NotificationPolicy::default();
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(1);

        let a = event("srv-a", Some("sess"), "k1", AgentEventKind::Completed, now);
        let b = event("srv-b", Some("sess"), "k1", AgentEventKind::Completed, now);

        assert!(policy.should_send(&a, started, now));
        assert!(policy.should_send(&b, started, now));
    }

    #[test]
    fn expired_entries_are_removed_and_capacity_is_bounded() {
        let ttl = Duration::from_secs(60);
        let capacity = 4;
        let mut policy = NotificationPolicy::new(ttl, capacity);
        let started = Utc::now();

        // Fill the map to capacity. All events occur strictly after the monitor start.
        for i in 0..capacity {
            let occurred_at = started + chrono::Duration::seconds(i as i64 + 1);
            let e = event(
                "srv",
                Some("sess"),
                &format!("k{}", i),
                AgentEventKind::Completed,
                occurred_at,
            );
            assert!(policy.should_send(&e, started, occurred_at));
        }

        // Exceeding capacity evicts the oldest entry.
        let newest_now = started + chrono::Duration::seconds(capacity as i64 + 2);
        let newest = event(
            "srv",
            Some("sess"),
            "k-new",
            AgentEventKind::Completed,
            newest_now,
        );
        assert!(policy.should_send(&newest, started, newest_now));

        // The oldest key was evicted, so sending it again is allowed.
        let oldest_again_now = started + chrono::Duration::seconds(capacity as i64 + 3);
        let oldest_again = event(
            "srv",
            Some("sess"),
            "k0",
            AgentEventKind::Completed,
            oldest_again_now,
        );
        assert!(policy.should_send(&oldest_again, started, oldest_again_now));

        // Capacity is never exceeded.
        assert!(policy.seen.len() <= capacity);

        // After the TTL all entries expire and can be re-sent.
        let after_ttl = started + chrono::Duration::seconds(200);
        let repeated = event(
            "srv",
            Some("sess"),
            "k1",
            AgentEventKind::Completed,
            after_ttl,
        );
        assert!(policy.should_send(&repeated, started, after_ttl));
    }

    #[test]
    fn baseline_events_are_not_sent() {
        let mut policy = NotificationPolicy::default();
        let started = Utc::now();

        // Exactly at the baseline is suppressed.
        let at_baseline = event(
            "srv",
            Some("sess"),
            "k1",
            AgentEventKind::Completed,
            started,
        );
        assert!(!policy.should_send(&at_baseline, started, started));

        // Before the baseline is suppressed.
        let before_baseline = event(
            "srv",
            Some("sess"),
            "k2",
            AgentEventKind::Failed,
            started - chrono::Duration::seconds(1),
        );
        assert!(!policy.should_send(&before_baseline, started, started));

        // After the baseline is allowed.
        let after_baseline = event(
            "srv",
            Some("sess"),
            "k3",
            AgentEventKind::Completed,
            started + chrono::Duration::seconds(1),
        );
        assert!(policy.should_send(
            &after_baseline,
            started,
            started + chrono::Duration::seconds(1)
        ));
    }

    #[test]
    fn only_four_kinds_map_to_notifications() {
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(1);
        let mut policy = NotificationPolicy::default();

        let kinds = [
            AgentEventKind::Completed,
            AgentEventKind::Failed,
            AgentEventKind::ApprovalRequired,
            AgentEventKind::QuestionRequired,
        ];

        for (i, kind) in kinds.iter().enumerate() {
            let e = event("srv", Some("sess"), &format!("k{}", i), kind.clone(), now);
            assert!(
                policy.should_send(&e, started, now),
                "{:?} should produce a notification",
                kind
            );
        }
    }

    #[test]
    fn coordinator_respects_settings() {
        let mut coord = NotificationCoordinator::new();
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(1);
        let evt = event("srv", Some("sess"), "k1", AgentEventKind::Completed, now);

        let disabled = DesktopSettings {
            notifications: false,
            ..Default::default()
        };
        assert!(coord
            .handle_event(&evt, &server(Backend::Kimi), started, now, &disabled)
            .is_none());

        let enabled = DesktopSettings::default();
        assert!(coord
            .handle_event(&evt, &server(Backend::Kimi), started, now, &enabled)
            .is_some());
    }

    #[test]
    fn title_mapping_matches_backend_and_kind() {
        let mut coord = NotificationCoordinator::new();
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(1);

        let kimi = event("srv", Some("sess"), "k1", AgentEventKind::Completed, now);
        let note = coord
            .handle_event(
                &kimi,
                &server(Backend::Kimi),
                started,
                now,
                &DesktopSettings::default(),
            )
            .unwrap();
        assert_eq!(note.title, "Kimi Code · 任务完成");

        let dsh = event(
            "srv",
            Some("sess"),
            "k2",
            AgentEventKind::ApprovalRequired,
            now,
        );
        let note = coord
            .handle_event(
                &dsh,
                &server(Backend::Dsh),
                started,
                now,
                &DesktopSettings::default(),
            )
            .unwrap();
        assert_eq!(note.title, "DeepSeek Harness · 等待审批");
    }

    #[test]
    fn notification_body_uses_body_or_session_title() {
        let mut coord = NotificationCoordinator::new();
        let started = Utc::now();
        let now = started + chrono::Duration::seconds(1);

        let mut with_body = event("srv", Some("sess"), "k1", AgentEventKind::Completed, now);
        with_body.body = Some("custom body".to_string());
        with_body.session_title = Some("title".to_string());
        let note = coord
            .handle_event(
                &with_body,
                &server(Backend::Kimi),
                started,
                now,
                &DesktopSettings::default(),
            )
            .unwrap();
        assert_eq!(note.body, "custom body");

        let mut without_body = event("srv", Some("sess"), "k2", AgentEventKind::Failed, now);
        without_body.body = None;
        without_body.session_title = Some("fallback title".to_string());
        let note = coord
            .handle_event(
                &without_body,
                &server(Backend::Kimi),
                started,
                now,
                &DesktopSettings::default(),
            )
            .unwrap();
        assert_eq!(note.body, "fallback title");
    }
}

//! generic 서비스 상태 — online + present(접속자) + max. source 가 갱신(sink),
//! SSE handler 가 `broadcast::Receiver` 로 변화를 받는다.
//!
//! MC 종속 없음. minecraft source 든 http-json source 든 같은 sink API 를 호출.
//! present 집합 변화를 diff 해 status_changed + join/leave notification 발행.

use std::collections::BTreeSet;
use std::sync::Arc;

use pengport_shared::psp::events::{NotificationEvent, NotificationLevel, ServiceEvent};
use pengport_shared::psp::status::{Metric, MetricType, Present, StatusResponse};
use serde_json::json;
use tokio::sync::{broadcast, RwLock};

/// 접속자 metric 라벨. generic("접속자" = 현재 연결된 사람). 서비스 무관.
const PRESENT_LABEL: &str = "접속자";

/// internal 상태. generic — 특정 서비스 종류에 종속되지 않음.
#[derive(Debug, Clone, Default)]
pub struct ServiceState {
    /// 서비스가 응답 가능한지.
    pub online: bool,
    /// 정원 (있으면). 없는 서비스는 None.
    pub max: Option<u32>,
    /// 현재 접속자 — service-native 신원(예: MC username).
    pub present: BTreeSet<String>,
}

impl ServiceState {
    pub fn new() -> Self {
        Self::default()
    }

    /// 현재 상태 → PSP StatusResponse.
    pub fn to_status(&self) -> StatusResponse {
        let names: Vec<String> = self.present.iter().cloned().collect();
        let players_value = json!({
            "online": names.len(),
            "max": self.max.unwrap_or(0),
            "names": names.clone(),
        });
        StatusResponse {
            online: self.online,
            metrics: vec![Metric {
                id: "players".to_string(),
                label: PRESENT_LABEL.to_string(),
                kind: MetricType::Players,
                value: players_value,
            }],
            badges: vec![],
            // presence(모임 레이어) — 접속자를 service-native 신원으로. client 가 칩 렌더.
            present: names
                .into_iter()
                .map(|name| Present {
                    id: name,
                    label: None,
                })
                .collect(),
            last_updated: None,
        }
    }
}

/// 호스트 전역 state.
pub struct AppState {
    pub state: RwLock<ServiceState>,
    /// PSP ServiceEvent broadcast. SSE 연결 수만큼 receiver.
    pub bus: broadcast::Sender<ServiceEvent>,
}

impl AppState {
    pub fn new(channel_capacity: usize) -> Arc<Self> {
        let (bus, _) = broadcast::channel(channel_capacity);
        Arc::new(Self {
            state: RwLock::new(ServiceState::new()),
            bus,
        })
    }

    /// 현재 status 스냅샷 (HTTP `/pengport/status` 응답).
    pub async fn current_status(&self) -> StatusResponse {
        self.state.read().await.to_status()
    }

    async fn emit_status_changed(&self) {
        let status = self.state.read().await.to_status();
        let _ = self.bus.send(ServiceEvent::StatusChanged(status));
    }

    fn notify(&self, title: String) {
        let _ = self.bus.send(ServiceEvent::Notification(NotificationEvent {
            level: NotificationLevel::Info,
            title,
            body: None,
        }));
    }

    /// 접속 적용. 새 이벤트면 status_changed + notification 발행.
    pub async fn present_join(&self, who: &str) {
        let changed = {
            let mut s = self.state.write().await;
            s.present.insert(who.to_string())
        };
        if changed {
            self.emit_status_changed().await;
            self.notify(format!("{} 님이 접속했습니다", who));
        }
    }

    pub async fn present_leave(&self, who: &str) {
        let changed = {
            let mut s = self.state.write().await;
            s.present.remove(who)
        };
        if changed {
            self.emit_status_changed().await;
            self.notify(format!("{} 님이 퇴장했습니다", who));
        }
    }

    /// authoritative 재동기화 — source 의 polling 결과로 drift 보정.
    pub async fn present_sync(&self, authoritative: BTreeSet<String>, max: Option<u32>) {
        let (was_offline, joined, left) = {
            let mut s = self.state.write().await;
            if let Some(m) = max {
                s.max = Some(m);
            }
            let was_offline = !s.online;
            s.online = true;
            let joined: Vec<String> = authoritative.difference(&s.present).cloned().collect();
            let left: Vec<String> = s.present.difference(&authoritative).cloned().collect();
            s.present = authoritative;
            (was_offline, joined, left)
        };

        if was_offline || !joined.is_empty() || !left.is_empty() {
            self.emit_status_changed().await;
        }
        for p in joined {
            self.notify(format!("{} 님이 접속했습니다", p));
        }
        for p in left {
            self.notify(format!("{} 님이 퇴장했습니다", p));
        }
    }

    pub async fn mark_offline(&self) {
        let was_online = {
            let mut s = self.state.write().await;
            if s.online {
                s.online = false;
                s.present.clear();
                true
            } else {
                false
            }
        };
        if was_online {
            self.emit_status_changed().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_status_present_mirrors_set() {
        let mut s = ServiceState::new();
        s.online = true;
        s.present.insert("alice".to_string());
        s.present.insert("bob".to_string());
        let status = s.to_status();
        assert_eq!(status.present.len(), 2);
        let ids: Vec<&str> = status.present.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"alice"));
        assert!(ids.contains(&"bob"));
        assert!(status.present.iter().all(|p| p.label.is_none()));
    }

    #[tokio::test]
    async fn join_and_leave_emit_status_changed() {
        let app = AppState::new(8);
        let mut rx = app.bus.subscribe();

        app.present_join("alice").await;
        match rx.recv().await.unwrap() {
            ServiceEvent::StatusChanged(s) => assert!(!s.metrics.is_empty()),
            other => panic!("expected status_changed first, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            ServiceEvent::Notification(n) => assert!(n.title.contains("alice")),
            other => panic!("expected notification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sync_marks_online() {
        let app = AppState::new(8);
        let mut rx = app.bus.subscribe();

        let mut auth = BTreeSet::new();
        auth.insert("bob".to_string());
        app.present_sync(auth, Some(4)).await;

        match rx.recv().await.unwrap() {
            ServiceEvent::StatusChanged(s) => assert!(s.online),
            _ => panic!(),
        }
    }
}

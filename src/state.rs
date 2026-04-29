//! 단일 MC 인스턴스 state. broadcaster 의 multi-instance 구조에서 단순화됨.
//!
//! Docker tail + RCON sync 가 같은 state 에 기록하고,
//! SSE handler 가 `broadcast::Receiver` 로 변화를 받는다.

use std::collections::BTreeSet;
use std::sync::Arc;

use pengport_shared::psp::events::{NotificationEvent, ServiceEvent};
use pengport_shared::psp::status::{Metric, MetricType, StatusResponse};
use serde_json::json;
use tokio::sync::{broadcast, RwLock};

/// internal state.
#[derive(Debug, Clone)]
pub struct McState {
    /// RCON 연결 가능한지 (= 컨테이너가 응답 가능한지).
    pub online: bool,
    pub max_players: u32,
    pub players: BTreeSet<String>,
}

impl McState {
    pub fn new() -> Self {
        Self {
            online: false,
            max_players: 0,
            players: BTreeSet::new(),
        }
    }

    /// 현재 state 를 PSP StatusResponse 로 변환.
    pub fn to_status(&self) -> StatusResponse {
        let players_value = json!({
            "online": self.players.len(),
            "max": self.max_players,
            "names": self.players.iter().cloned().collect::<Vec<_>>(),
        });
        StatusResponse {
            online: self.online,
            metrics: vec![Metric {
                id: "players".to_string(),
                label: "접속자".to_string(),
                kind: MetricType::Players,
                value: players_value,
            }],
            badges: vec![],
            last_updated: None,
        }
    }
}

impl Default for McState {
    fn default() -> Self {
        Self::new()
    }
}

/// 어댑터 전역 state.
pub struct AppState {
    pub state: RwLock<McState>,
    /// PSP ServiceEvent broadcast. SSE 연결 수만큼 receiver.
    pub bus: broadcast::Sender<ServiceEvent>,
}

impl AppState {
    pub fn new(channel_capacity: usize) -> Arc<Self> {
        let (bus, _) = broadcast::channel(channel_capacity);
        Arc::new(Self {
            state: RwLock::new(McState::new()),
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

    /// 플레이어 join 적용. 새 이벤트면 status_changed + notification 발행.
    pub async fn apply_join(&self, player: &str) {
        let changed = {
            let mut s = self.state.write().await;
            s.players.insert(player.to_string())
        };
        if changed {
            self.emit_status_changed().await;
            let _ = self.bus.send(ServiceEvent::Notification(NotificationEvent {
                level: pengport_shared::psp::events::NotificationLevel::Info,
                title: format!("{} 님이 접속했습니다", player),
                body: None,
            }));
        }
    }

    pub async fn apply_leave(&self, player: &str) {
        let changed = {
            let mut s = self.state.write().await;
            s.players.remove(player)
        };
        if changed {
            self.emit_status_changed().await;
            let _ = self.bus.send(ServiceEvent::Notification(NotificationEvent {
                level: pengport_shared::psp::events::NotificationLevel::Info,
                title: format!("{} 님이 퇴장했습니다", player),
                body: None,
            }));
        }
    }

    /// RCON 결과로 authoritative 재동기화 — broadcaster 의 같은 함수와 동일 의미.
    pub async fn sync_from_authoritative(
        &self,
        authoritative: BTreeSet<String>,
        max_players: Option<u32>,
    ) {
        let (was_offline, joined, left) = {
            let mut s = self.state.write().await;
            if let Some(m) = max_players {
                s.max_players = m;
            }
            let was_offline = !s.online;
            s.online = true;
            let joined: Vec<String> = authoritative.difference(&s.players).cloned().collect();
            let left: Vec<String> = s.players.difference(&authoritative).cloned().collect();
            s.players = authoritative;
            (was_offline, joined, left)
        };

        if was_offline || !joined.is_empty() || !left.is_empty() {
            self.emit_status_changed().await;
        }
        for p in joined {
            let _ = self.bus.send(ServiceEvent::Notification(NotificationEvent {
                level: pengport_shared::psp::events::NotificationLevel::Info,
                title: format!("{} 님이 접속했습니다", p),
                body: None,
            }));
        }
        for p in left {
            let _ = self.bus.send(ServiceEvent::Notification(NotificationEvent {
                level: pengport_shared::psp::events::NotificationLevel::Info,
                title: format!("{} 님이 퇴장했습니다", p),
                body: None,
            }));
        }
    }

    pub async fn mark_offline(&self) {
        let was_online = {
            let mut s = self.state.write().await;
            if s.online {
                s.online = false;
                s.players.clear();
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

    #[tokio::test]
    async fn join_and_leave_emit_status_changed() {
        let app = AppState::new(8);
        let mut rx = app.bus.subscribe();

        app.apply_join("alice").await;
        // 첫 이벤트는 status_changed
        match rx.recv().await.unwrap() {
            ServiceEvent::StatusChanged(s) => {
                assert!(!s.metrics.is_empty());
            }
            other => panic!("expected status_changed first, got {other:?}"),
        }
        // 다음은 notification
        match rx.recv().await.unwrap() {
            ServiceEvent::Notification(n) => assert!(n.title.contains("alice")),
            other => panic!("expected notification, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rcon_sync_marks_online() {
        let app = AppState::new(8);
        let mut rx = app.bus.subscribe();

        let mut auth = BTreeSet::new();
        auth.insert("bob".to_string());
        app.sync_from_authoritative(auth, Some(4)).await;

        match rx.recv().await.unwrap() {
            ServiceEvent::StatusChanged(s) => {
                assert!(s.online);
            }
            _ => panic!(),
        }
    }
}

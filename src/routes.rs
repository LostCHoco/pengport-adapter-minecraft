//! PSP HTTP endpoints + SSE handler.
//!
//! - `GET /.well-known/pengport-service` → ServiceManifest (JSON)
//! - `GET /pengport/status` → StatusResponse (JSON)
//! - `GET /pengport/events?token=...` (SSE) → ServiceEvent stream

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    Json,
};
use futures_util::stream::{self, Stream, StreamExt};
use pengport_shared::psp::events::ServiceEvent;
use pengport_shared::psp::manifest::ServiceManifest;
use pengport_shared::psp::status::StatusResponse;
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio_stream::wrappers::BroadcastStream;

use crate::config::SecretString;
use crate::state::AppState;

#[derive(Clone)]
pub struct AppCtx {
    pub state: Arc<AppState>,
    pub manifest: ServiceManifest,
    pub events_token: Option<SecretString>,
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn manifest_handler(State(ctx): State<AppCtx>) -> Json<ServiceManifest> {
    Json(ctx.manifest.clone())
}

pub async fn status_handler(State(ctx): State<AppCtx>) -> Json<StatusResponse> {
    Json(ctx.state.current_status().await)
}

/// Constant-time 토큰 비교 (timing attack 방어).
fn ct_token_eq(provided: &[u8], expected: &[u8]) -> bool {
    if provided.len() != expected.len() {
        let _ = expected.ct_eq(expected);
        return false;
    }
    provided.ct_eq(expected).into()
}

pub async fn events_handler(
    State(ctx): State<AppCtx>,
    Query(q): Query<TokenQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, StatusCode> {
    if let Some(expected) = &ctx.events_token {
        let provided = q.token.as_deref().unwrap_or("").as_bytes();
        if !ct_token_eq(provided, expected.expose().as_bytes()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    let rx = ctx.state.bus.subscribe();
    let initial_status = ctx.state.current_status().await;
    let initial = stream::once(async move {
        event_to_sse(&ServiceEvent::StatusChanged(initial_status))
    });

    let bcast = BroadcastStream::new(rx).filter_map(|res| async move {
        match res {
            Ok(ev) => Some(event_to_sse(&ev)),
            Err(e) => {
                tracing::warn!(?e, "SSE 구독자 지연(lag)");
                None
            }
        }
    });

    Ok(Sse::new(initial.chain(bcast)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text(":keepalive"),
    ))
}

fn event_to_sse(ev: &ServiceEvent) -> Result<SseEvent, Infallible> {
    let (name, data) = match ev {
        ServiceEvent::StatusChanged(status) => (
            "status_changed",
            serde_json::to_string(status).unwrap_or_else(|_| "{}".to_string()),
        ),
        ServiceEvent::Notification(n) => (
            "notification",
            serde_json::to_string(n).unwrap_or_else(|_| "{}".to_string()),
        ),
        ServiceEvent::Custom(c) => (
            "custom",
            serde_json::to_string(c).unwrap_or_else(|_| "{}".to_string()),
        ),
    };
    Ok(SseEvent::default().event(name).data(data))
}

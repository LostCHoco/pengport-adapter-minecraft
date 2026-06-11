//! `pengport-adapter-core` — PengPort PSP 어댑터 generic 호스트 라이브러리.
//!
//! 무거운 공통부(PSP 서버 + ServiceState + SSE/auth)를 담아 모든 flavor 바이너리가
//! embed. flavor(minecraft 등)는 source 로직 + manifest 내용만 더해 thin 바이너리로.
//!
//! flavor 바이너리 흐름:
//! 1. `AppState::new(cap)` 로 sink 생성
//! 2. source 루프(예: minecraft 의 log tail + rcon) spawn → sink(`present_join` 등) 갱신
//! 3. `AppCtx { state, manifest, events_token }` 구성
//! 4. `serve(bind, ctx).await`

pub mod config;
pub mod routes;
pub mod state;

pub use config::SecretString;
pub use routes::AppCtx;
pub use state::{AppState, ServiceState};

use anyhow::Result;
use axum::{routing::get, Router};
use tokio::net::TcpListener;

/// PSP 라우터 구성 (manifest/status/events). 서비스 종류 무관.
pub fn build_router(ctx: AppCtx) -> Router {
    Router::new()
        .route(
            "/.well-known/pengport-service",
            get(routes::manifest_handler),
        )
        .route("/pengport/status", get(routes::status_handler))
        .route("/pengport/events", get(routes::events_handler))
        .with_state(ctx)
}

/// bind 주소에서 PSP 서버 serve. flavor 가 source 루프 spawn 후 호출.
pub async fn serve(bind: &str, ctx: AppCtx) -> Result<()> {
    let app = build_router(ctx);
    let listener = TcpListener::bind(bind).await?;
    tracing::info!("HTTP listen: http://{}", bind);
    axum::serve(listener, app).await?;
    Ok(())
}

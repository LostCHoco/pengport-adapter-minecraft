//! PengPort MC 어댑터.
//!
//! 단일 Minecraft 컨테이너를 PSP service 로 외부 노출.
//!
//! ```text
//! [Minecraft 컨테이너]
//!     │ Docker logs (-f) + RCON list
//!     ▼
//! [adapter-minecraft]  ← 이 바이너리
//!     │ /.well-known/pengport-service  (manifest)
//!     │ /pengport/status                (StatusResponse)
//!     │ /pengport/events                (SSE: ServiceEvent)
//!     ▼
//! [PengPort client / broadcaster]
//! ```

mod config;
mod docker_tail;
mod manifest;
mod parser;
mod rcon;
mod routes;
mod state;

use std::time::Duration;

use anyhow::Result;
use axum::{routing::get, Router};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::AppConfig;
use crate::docker_tail::ContainerEvent;
use crate::parser::PlayerEvent;
use crate::routes::AppCtx;
use crate::state::AppState;

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = AppConfig::from_env()?;
    tracing::info!(
        mc_id = %cfg.mc_id,
        container = %cfg.container,
        bind = %cfg.bind,
        "adapter-minecraft 시작"
    );

    let state = AppState::new(256);
    let docker = docker_tail::connect_local()?;

    // 1. 컨테이너 이벤트 채널.
    let (evt_tx, mut evt_rx) = mpsc::channel::<ContainerEvent>(1024);

    // 2. Docker tail.
    tokio::spawn(docker_tail::tail_with_reconnect(
        docker.clone(),
        cfg.container.clone(),
        evt_tx.clone(),
    ));

    // 3. RCON sync (drift 보정).
    tokio::spawn(rcon::rcon_sync_loop(
        state.clone(),
        cfg.rcon_address.clone(),
        cfg.rcon_password.expose().to_string(),
        Duration::from_secs(30),
        Duration::from_secs(120),
    ));
    drop(evt_tx);

    // 4. ContainerEvent → AppState dispatcher.
    let state_for_disp = state.clone();
    tokio::spawn(async move {
        while let Some(ev) = evt_rx.recv().await {
            match ev {
                ContainerEvent::Player(PlayerEvent::Join(p)) => {
                    state_for_disp.apply_join(&p).await
                }
                ContainerEvent::Player(PlayerEvent::Leave(p)) => {
                    state_for_disp.apply_leave(&p).await
                }
                ContainerEvent::StreamEnded => {
                    tracing::info!("docker tail 스트림 종료");
                }
            }
        }
    });

    // 5. PSP manifest (어댑터 부팅 시 1회 빌드, 자체 base_url 결정).
    //    base_url 결정: PSP_PUBLIC_BASE_URL 환경변수 (publish 도메인) 우선, 없으면 BIND.
    let base_url = std::env::var("PSP_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("http://{}", cfg.bind));
    let manifest = manifest::build_manifest(&cfg, &base_url);

    let ctx = AppCtx {
        state: state.clone(),
        manifest,
        events_token: cfg.events_token.clone(),
    };

    let app = Router::new()
        .route("/.well-known/pengport-service", get(routes::manifest_handler))
        .route("/pengport/status", get(routes::status_handler))
        .route("/pengport/events", get(routes::events_handler))
        .with_state(ctx);

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!("HTTP listen: http://{}", cfg.bind);
    axum::serve(listener, app).await?;

    Ok(())
}

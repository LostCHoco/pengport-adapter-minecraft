//! PengPort 어댑터 — Minecraft flavor.
//!
//! core(generic 호스트) 위에 minecraft source(log tail + RCON)를 얹은 thin 바이너리.
//!
//! ```text
//! [Minecraft 컨테이너] ── /data 를 host bind mount
//!     │                       │
//!     │ RCON list (drift 보정)  │ logs/latest.log (host filesystem)
//!     ▼                       ▼
//! [minecraft source] ──push/pull──▶ core AppState(sink)
//!     │
//! [core PSP 서버] ── /.well-known/pengport-service · /pengport/status · /pengport/events
//! ```

mod log_tail;
mod manifest;
mod mc_config;
mod parser;
mod rcon;

use std::time::Duration;

use anyhow::Result;
use pengport_adapter_core::{serve, AppCtx, AppState};
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::log_tail::ContainerEvent;
use crate::mc_config::AppConfig;
use crate::parser::PlayerEvent;

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
        log_dir = %cfg.log_dir.display(),
        bind = %cfg.bind,
        "adapter-minecraft 시작"
    );

    // core sink.
    let state = AppState::new(256);

    // minecraft source — 1. 컨테이너 이벤트 채널.
    let (evt_tx, mut evt_rx) = mpsc::channel::<ContainerEvent>(1024);

    // 2. 로그 file watch (inotify, push). sub-second.
    let log_dir = cfg.log_dir.clone();
    let evt_tx_clone = evt_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = log_tail::watch_logs(log_dir, evt_tx_clone).await {
            tracing::error!(error = %e, "log_tail watch 시작 실패");
        }
    });

    // 3. RCON sync (drift 보정, pull). 30s / unhealthy 60s.
    tokio::spawn(rcon::rcon_sync_loop(
        state.clone(),
        cfg.rcon_address.clone(),
        cfg.rcon_password.expose().to_string(),
        Duration::from_secs(30),
        Duration::from_secs(60),
    ));
    drop(evt_tx);

    // 4. ContainerEvent → core sink dispatcher.
    let state_for_disp = state.clone();
    tokio::spawn(async move {
        while let Some(ev) = evt_rx.recv().await {
            match ev {
                ContainerEvent::Player(PlayerEvent::Join(p)) => {
                    state_for_disp.present_join(&p).await
                }
                ContainerEvent::Player(PlayerEvent::Leave(p)) => {
                    state_for_disp.present_leave(&p).await
                }
                ContainerEvent::StreamEnded => {
                    tracing::info!("log_tail stream 종료");
                }
            }
        }
    });

    // 5. PSP manifest (minecraft 내용) — base_url: PSP_PUBLIC_BASE_URL 우선, 없으면 BIND.
    let base_url =
        std::env::var("PSP_PUBLIC_BASE_URL").unwrap_or_else(|_| format!("http://{}", cfg.bind));
    let manifest = manifest::build_manifest(&cfg, &base_url);

    let ctx = AppCtx {
        state: state.clone(),
        manifest,
        events_token: cfg.events_token.clone(),
    };

    // 6. core PSP 서버 serve.
    serve(&cfg.bind, ctx).await
}

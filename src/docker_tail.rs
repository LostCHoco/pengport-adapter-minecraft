//! Docker Engine API 로 컨테이너 로그를 `-f` 스트림으로 읽어 플레이어 이벤트를 추출.
//!
//! 단일 MC 컨테이너만 추적 (multi-instance 는 별도 어댑터 프로세스).
//! 연결이 끊기면 지수 백오프로 재연결 시도.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bollard::{query_parameters::LogsOptions, Docker};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::parser::{parse_line, PlayerEvent};

fn now_unix_secs() -> i32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i32)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub enum ContainerEvent {
    Player(PlayerEvent),
    StreamEnded,
}

/// 기본 Docker 연결. Unix socket.
///
/// `connect_with_local_defaults()` 는 default timeout 을 적용하는데, 그게 logs API
/// 같은 long-running stream 에 발동해 데이터 idle 시 stream close 를 유발.
/// timeout=0 은 0초 timeout (즉시 timeout) 으로 해석되므로 매우 큰 값 (1년) 명시.
const STREAM_TIMEOUT_SECS: u64 = 365 * 24 * 60 * 60; // 1년

pub fn connect_local() -> Result<Docker> {
    Docker::connect_with_socket(
        "/var/run/docker.sock",
        STREAM_TIMEOUT_SECS,
        bollard::API_DEFAULT_VERSION,
    )
    .context("Docker Engine 에 연결 실패")
}

pub async fn tail_container(
    docker: Docker,
    container_name: String,
    out: mpsc::Sender<ContainerEvent>,
) -> Result<()> {
    // tail="0" + follow=true 시 일부 docker engine 빌드에서 stream 이 즉시 close 되는
    // 현상을 회피. since 를 현재 시각으로 설정하면 docker API 가 "현재 이후 새 로그만"
    // 으로 해석 → tail 미지정 (default = "all") 도 안전하게 새 로그만 follow.
    let options = Some(LogsOptions {
        follow: true,
        stdout: true,
        stderr: true,
        since: now_unix_secs(),
        timestamps: false,
        ..Default::default()
    });

    let mut stream = docker.logs(&container_name, options);
    let mut buffer = String::new();

    while let Some(item) = stream.next().await {
        let chunk = match item {
            Ok(output) => output.to_string(),
            Err(e) => {
                tracing::warn!(error=%e, "Docker log 스트림 에러");
                break;
            }
        };

        buffer.push_str(&chunk);

        while let Some(nl) = buffer.find('\n') {
            let line = buffer[..nl].trim_end_matches('\r').to_string();
            buffer.drain(..=nl);

            if let Some(event) = parse_line(&line) {
                tracing::debug!(?event, "플레이어 이벤트");
                if out.send(ContainerEvent::Player(event)).await.is_err() {
                    return Ok(());
                }
            }
        }
    }

    tracing::info!("로그 스트림 종료");
    let _ = out.send(ContainerEvent::StreamEnded).await;
    Ok(())
}

pub async fn tail_with_reconnect(
    docker: Docker,
    container_name: String,
    out: mpsc::Sender<ContainerEvent>,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        match tail_container(docker.clone(), container_name.clone(), out.clone()).await {
            Ok(_) => {
                tracing::info!("정상 종료 → 재연결 대기 {:?}", backoff);
            }
            Err(e) => {
                tracing::warn!(error=%e, "tail 실패 → 재연결 대기 {:?}", backoff);
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

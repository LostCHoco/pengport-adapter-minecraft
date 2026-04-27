//! Docker Engine API 로 컨테이너 로그를 `-f` 스트림으로 읽어 플레이어 이벤트를 추출.
//!
//! 단일 MC 컨테이너만 추적 (multi-instance 는 별도 어댑터 프로세스).
//! 연결이 끊기면 지수 백오프로 재연결 시도.

use std::time::Duration;

use anyhow::{Context, Result};
use bollard::{query_parameters::LogsOptions, Docker};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::parser::{parse_line, PlayerEvent};

#[derive(Debug, Clone)]
pub enum ContainerEvent {
    Player(PlayerEvent),
    StreamEnded,
}

/// 기본 Docker 연결. Unix socket (Linux) / named pipe (Windows).
pub fn connect_local() -> Result<Docker> {
    Docker::connect_with_local_defaults().context("Docker Engine 에 연결 실패")
}

pub async fn tail_container(
    docker: Docker,
    container_name: String,
    out: mpsc::Sender<ContainerEvent>,
) -> Result<()> {
    let options = Some(LogsOptions {
        follow: true,
        stdout: true,
        stderr: true,
        tail: "0".to_string(),
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

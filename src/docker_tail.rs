//! Docker CLI (`docker logs -f`) subprocess 로 컨테이너 로그를 실시간 스트리밍.
//!
//! 구현 배경: bollard 0.19 의 `logs()` stream 이 매 1~30초마다 즉시 None 으로 종료되어
//! follow stream 유지 안 됨. 여러 fix 시도 (tail=0, since=now, timeout=1년 등) 후에도
//! 해결 못 함. docker engine 자체는 `docker logs -f` 로 stream 정상 유지하므로 docker
//! CLI 를 subprocess 로 spawn 해서 stdout 을 line-buffered 로 읽음. 검증된 동작.
//!
//! adapter container 에는 docker-cli 가 설치되어 있어야 함 (Dockerfile 참조).
//! /var/run/docker.sock 은 read-only mount.
//!
//! 단일 MC 컨테이너만 추적 (multi-instance 는 별도 어댑터 프로세스).
//! 연결이 끊기면 빠른 재연결 + 에러 시 지수 백오프.

use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task;

use crate::parser::{parse_line, PlayerEvent};

#[derive(Debug, Clone)]
pub enum ContainerEvent {
    Player(PlayerEvent),
    StreamEnded,
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `docker logs -f --since=<now> <container>` 실행. stdout 을 line-buffered 로 읽어
/// player join/leave 이벤트로 파싱 후 채널에 전송. process 가 종료되면 Ok 반환
/// (호출자가 reconnect 결정).
pub async fn tail_container(
    container_name: String,
    out: mpsc::Sender<ContainerEvent>,
) -> Result<()> {
    // since 를 35초 전으로 — docker engine 이 ~30초 idle 시 stream 종료하는 known quirk
    // 가 있어 reconnect 시 그 사이 갭의 logs (player join/leave 등) 를 못 받는 문제 회피.
    // 35초 history + follow 모드. parser 가 이미 본 line 은 dedup 안 하지만 player event
    // apply 자체는 idempotent (state 의 join/leave 동일 player 처리는 안전).
    //
    // `--since=값` 합친 형식 — `--since` `값` 분리 형식은 일부 환경에서 stream 즉시
    // 종료하는 docker CLI quirk 가 있음. 합친 형식이 안정.
    let since_arg = format!("--since={}", now_unix_secs().saturating_sub(35));
    let mut child = Command::new("docker")
        .args(["logs", "-f", &since_arg, &container_name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("docker logs spawn 실패 (docker CLI 가 PATH 에 있어야 함)")?;

    let stdout = child
        .stdout
        .take()
        .context("docker logs subprocess stdout pipe 미존재")?;
    let stderr = child
        .stderr
        .take()
        .context("docker logs subprocess stderr pipe 미존재")?;

    // stderr 는 background 로 읽어 warn 로깅 — docker engine 의 에러 메시지 진단용
    task::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::warn!(stderr_line = %line, "docker logs stderr");
        }
    });

    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines
        .next_line()
        .await
        .context("docker logs stdout read 실패")?
    {
        if let Some(event) = parse_line(&line) {
            tracing::debug!(?event, "플레이어 이벤트");
            if out.send(ContainerEvent::Player(event)).await.is_err() {
                break;
            }
        }
    }

    let status = child.wait().await.context("docker logs wait 실패")?;
    tracing::info!(exit = ?status.code(), "로그 스트림 종료");
    let _ = out.send(ContainerEvent::StreamEnded).await;
    Ok(())
}

pub async fn tail_with_reconnect(container_name: String, out: mpsc::Sender<ContainerEvent>) {
    // CLI subprocess 가 정상 종료되면 (e.g., container restart) 빠른 reconnect.
    // 진짜 에러 (CLI 실행 실패, container 없음 등) 는 지수 백오프.
    const QUICK_RECONNECT: Duration = Duration::from_secs(1);
    const ERROR_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
    const ERROR_BACKOFF_MAX: Duration = Duration::from_secs(60);

    let mut error_backoff = ERROR_BACKOFF_INITIAL;

    loop {
        match tail_container(container_name.clone(), out.clone()).await {
            Ok(_) => {
                error_backoff = ERROR_BACKOFF_INITIAL;
                tokio::time::sleep(QUICK_RECONNECT).await;
            }
            Err(e) => {
                tracing::warn!(error=%e, "tail 실패 → 재연결 대기 {:?}", error_backoff);
                tokio::time::sleep(error_backoff).await;
                error_backoff = (error_backoff * 2).min(ERROR_BACKOFF_MAX);
            }
        }
    }
}

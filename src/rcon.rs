//! RCON 을 통해 서버의 현재 접속자 목록을 주기적으로 조회해 internal state 와
//! 동기화한다. Docker tail 이 놓친 이벤트를 보정.
//!
//! RCON `list` 응답:
//!   "There are X of a max of Y players online: name1, name2, name3"
//!
//! 두 포맷 지원: Fabric/Paper 1.21+ ("of a max of"), Forge 1.12.2 ("X/Y").

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result};
use rcon::Connection;
use regex::Regex;
use tokio::net::TcpStream;

use crate::state::AppState;

pub fn parse_list_response(response: &str) -> Option<(u32, u32, BTreeSet<String>)> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"There are (\d+)(?:/| of a max of )(\d+) players online:?\s*(.*)").unwrap()
    });

    let caps = RE.captures(response.trim())?;
    let count: u32 = caps[1].parse().ok()?;
    let max: u32 = caps[2].parse().ok()?;
    let names_str = caps[3].trim();
    let players: BTreeSet<String> = if names_str.is_empty() {
        BTreeSet::new()
    } else {
        names_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    Some((count, max, players))
}

pub async fn rcon_list(address: &str, password: &str) -> Result<(u32, u32, BTreeSet<String>)> {
    let mut conn = Connection::<TcpStream>::builder()
        .enable_minecraft_quirks(true)
        .connect(address, password)
        .await
        .context("RCON 접속 실패")?;

    let response = conn.cmd("list").await.context("list 명령 실패")?;
    parse_list_response(&response).context("list 응답 파싱 실패")
}

pub async fn rcon_sync_loop(
    state: Arc<AppState>,
    address: String,
    password: String,
    interval: Duration,
    offline_threshold: Duration,
) {
    let mut last_success = tokio::time::Instant::now();
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;
        match rcon_list(&address, &password).await {
            Ok((_count, max, players)) => {
                state.sync_from_authoritative(players, Some(max)).await;
                last_success = tokio::time::Instant::now();
            }
            Err(e) => {
                tracing::warn!("RCON 호출 실패");
                tracing::debug!(error=%e, "RCON detail");
                if last_success.elapsed() >= offline_threshold {
                    state.mark_offline().await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_populated_list() {
        let resp = "There are 2 of a max of 4 players online: alice, bob";
        let (count, max, players) = parse_list_response(resp).unwrap();
        assert_eq!(count, 2);
        assert_eq!(max, 4);
        assert!(players.contains("alice"));
        assert!(players.contains("bob"));
    }

    #[test]
    fn parses_empty_list() {
        let resp = "There are 0 of a max of 4 players online:";
        let (_, max, players) = parse_list_response(resp).unwrap();
        assert_eq!(max, 4);
        assert!(players.is_empty());
    }

    #[test]
    fn parses_forge_1_12_format() {
        let resp = "There are 0/4 players online:";
        let (count, max, players) = parse_list_response(resp).unwrap();
        assert_eq!(count, 0);
        assert_eq!(max, 4);
        assert!(players.is_empty());
    }

    #[test]
    fn rejects_noise() {
        assert!(parse_list_response("unrelated text").is_none());
    }
}

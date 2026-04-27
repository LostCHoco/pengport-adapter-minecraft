//! Minecraft 서버 로그 라인에서 플레이어 접속/퇴장 이벤트를 추출한다.
//!
//! 지원 포맷 (Fabric 1.21.1, Forge 1.12.2 공통):
//! - `[HH:MM:SS] [Server thread/INFO] [...]: <Name> joined the game`
//! - `[HH:MM:SS] [Server thread/INFO] [...]: <Name> left the game`
//!
//! Docker / itzg 이미지는 앞에 `> [K[...]` 같은 ANSI/제어 문자가 붙는 경우가 있어,
//! `Server thread` 이후 부분만 기준으로 잡는다.

use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    Join(String),
    Leave(String),
}

/// 한 줄에서 이벤트를 추출. 매치 없으면 `None`.
pub fn parse_line(line: &str) -> Option<PlayerEvent> {
    static JOIN_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b([A-Za-z0-9_]{1,16})\s+joined the game\b").unwrap()
    });
    static LEAVE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b([A-Za-z0-9_]{1,16})\s+left the game\b").unwrap()
    });

    if let Some(caps) = JOIN_RE.captures(line) {
        return Some(PlayerEvent::Join(caps[1].to_string()));
    }
    if let Some(caps) = LEAVE_RE.captures(line) {
        return Some(PlayerEvent::Leave(caps[1].to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fabric_join() {
        let line = "[20:41:22] [Server thread/INFO] [minecraft/DedicatedServer]: pengport joined the game";
        assert_eq!(parse_line(line), Some(PlayerEvent::Join("pengport".into())));
    }

    #[test]
    fn parses_fabric_leave() {
        let line = "[20:42:55] [Server thread/INFO] [minecraft/DedicatedServer]: pengport left the game";
        assert_eq!(parse_line(line), Some(PlayerEvent::Leave("pengport".into())));
    }

    #[test]
    fn parses_forge_join() {
        let line = "[00:52:14] [Server thread/INFO] [net.minecraft.server.MinecraftServer]: Alice123 joined the game";
        assert_eq!(parse_line(line), Some(PlayerEvent::Join("Alice123".into())));
    }

    #[test]
    fn ignores_generic_lines() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("[INFO]: Starting minecraft server"), None);
    }

    #[test]
    fn rejects_too_long_name() {
        let line = "[INFO]: ThisNameIsTooLongForMinecraft joined the game";
        assert_eq!(parse_line(line), None);
    }

    #[test]
    fn handles_ansi_prefix() {
        let line = "> \u{001b}[K[20:41:22] [Server thread/INFO]: pengport joined the game";
        assert_eq!(parse_line(line), Some(PlayerEvent::Join("pengport".into())));
    }
}

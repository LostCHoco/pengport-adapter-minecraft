//! 환경변수 기반 어댑터 설정.
//!
//! 단일 MC 인스턴스 = 단일 어댑터 컨테이너. multi-instance 면 여러 어댑터 띄움.
//!
//! ## 필수
//! - `MC_ID`              — service id (catalog 와 일치). 예: `modded-mc`
//! - `MC_NAME`            — manifest 의 사용자 표시 이름. 예: `알파펭`
//! - `MC_LOG_DIR`         — minecraft logs 디렉토리 (host bind mount → adapter ro).
//!   예: `/mc-logs` (latest.log 가 이 경로 안에 있어야 함)
//! - `RCON_ADDRESS`       — `host:port` (컨테이너 네트워크 내)
//! - `RCON_PASSWORD`      — RCON 비밀번호
//! - `MC_HOST`            — 클라이언트가 접속할 도메인/IP (public)
//! - `MC_PORT`            — 클라이언트가 접속할 포트 (default 25565)
//! - `MC_VERSION`         — Minecraft 버전 (예: `1.21.1`)
//! - `MC_LOADER`          — `vanilla` | `fabric` | `forge` | `neoforge` | `quilt`
//!
//! ## 선택
//! - `BIND`               — HTTP 리슨 (default `0.0.0.0:8080`)
//! - `MC_LOADER_VERSION`  — vanilla 외에는 필수
//! - `PACKWIZ_URL`        — packwiz pack.toml URL (모드팩 자동 동기화)
//! - `MC_DISPLAY_NAME`    — Prism 인스턴스 이름 (default = MC_NAME)
//! - `MC_JAVA_MAJOR`      — Java major (예: 21). 없으면 client 가 Prism default 사용
//! - `MC_DESCRIPTION`     — manifest description
//! - `MC_ICON_URL`        — manifest icon
//! - `EVENTS_TOKEN`       — events SSE 인증 토큰. 비어있으면 인증 없음

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Secret string with masking on Debug/Display.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"***\"")
    }
}

impl std::fmt::Display for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: String,
    pub events_token: Option<SecretString>,

    // service identity
    pub mc_id: String,
    pub mc_name: String,
    pub mc_description: Option<String>,
    pub mc_icon_url: Option<String>,

    // log file tail + RCON
    pub log_dir: PathBuf,
    pub rcon_address: String,
    pub rcon_password: SecretString,

    // PSP action (prism-launcher config)
    pub mc_host: String,
    pub mc_port: u16,
    pub mc_version: String,
    pub mc_loader: String,
    pub mc_loader_version: Option<String>,
    pub mc_display_name: Option<String>,
    pub mc_java_major: Option<u32>,
    pub packwiz_url: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let bind = env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let events_token = env::var("EVENTS_TOKEN").ok().map(SecretString::new);

        let mc_id = env::var("MC_ID").context("MC_ID 누락")?;
        let mc_name = env::var("MC_NAME").context("MC_NAME 누락")?;
        let mc_description = env::var("MC_DESCRIPTION").ok();
        let mc_icon_url = env::var("MC_ICON_URL").ok();

        let log_dir: PathBuf = env::var("MC_LOG_DIR")
            .context("MC_LOG_DIR 누락 — minecraft 의 logs 디렉토리를 ro mount 한 경로")?
            .into();
        let rcon_address = env::var("RCON_ADDRESS").context("RCON_ADDRESS 누락")?;
        let rcon_password = env::var("RCON_PASSWORD")
            .context("RCON_PASSWORD 누락")
            .map(SecretString::new)?;

        let mc_host = env::var("MC_HOST").context("MC_HOST 누락")?;
        let mc_port: u16 = env::var("MC_PORT")
            .unwrap_or_else(|_| "25565".to_string())
            .parse()
            .context("MC_PORT 파싱 실패")?;
        let mc_version = env::var("MC_VERSION").context("MC_VERSION 누락")?;
        let mc_loader = env::var("MC_LOADER").context("MC_LOADER 누락")?;
        let mc_loader_version = env::var("MC_LOADER_VERSION").ok();
        let mc_display_name = env::var("MC_DISPLAY_NAME").ok();
        let mc_java_major = env::var("MC_JAVA_MAJOR")
            .ok()
            .map(|s| s.parse::<u32>().context("MC_JAVA_MAJOR 파싱 실패"))
            .transpose()?;
        let packwiz_url = env::var("PACKWIZ_URL").ok();

        Ok(Self {
            bind,
            events_token,
            mc_id,
            mc_name,
            mc_description,
            mc_icon_url,
            log_dir,
            rcon_address,
            rcon_password,
            mc_host,
            mc_port,
            mc_version,
            mc_loader,
            mc_loader_version,
            mc_display_name,
            mc_java_major,
            packwiz_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_masks() {
        let s = SecretString::new("super-secret".into());
        assert_eq!(format!("{:?}", s), "\"***\"");
        assert_eq!(s.expose(), "super-secret");
    }
}

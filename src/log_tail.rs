//! Minecraft `latest.log` 파일을 inotify 로 직접 watch — docker engine 우회.
//!
//! 배경: `docker logs -f` 로 컨테이너 stdout 을 follow 하던 이전 구현은 docker engine 이
//! ~30s idle 시 stream 을 server-side close 하는 quirk 가 있어 player join/leave 가
//! reconnect 갭에 떨어지면 누락. itzg/minecraft-server 이미지가 `/data` 를 host bind
//! mount 하므로 logs 도 host filesystem 에서 직접 접근 가능. inotify 로 file change
//! event 받아 sub-second 에 push.
//!
//! ## 처리 케이스
//!
//! 1. **시작 시 latest.log 부재** — server stopped/autopaused. 디렉토리 watch 만으로
//!    파일 등장 대기 (busy-poll 없음).
//! 2. **회전 (자정/재시작)** — `latest.log` → `YYYY-MM-DD-N.log` rename 후 새 latest.log
//!    생성. inode 변화로 감지 → 새 fd reopen.
//! 3. **truncate** — file size < 우리의 read pos 이면 reopen at start.
//! 4. **append (정상)** — Modify event → 다음 라인까지 read.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;

use crate::parser::{parse_line, PlayerEvent};

#[derive(Debug, Clone)]
pub enum ContainerEvent {
    Player(PlayerEvent),
    /// 호환성 유지용 — 현재 file watch 모델에서는 거의 발생하지 않지만 main.rs 의
    /// match arm 이 있고, 추후 unrecoverable 한 watcher 에러를 알릴 채널로 쓸 수 있음.
    #[allow(dead_code)]
    StreamEnded,
}

const FILE_NAME: &str = "latest.log";
/// notify 의 missed event 보정용 fallback poll. 정상 흐름에서는 inotify event
/// 가 즉시 wake 시키므로 이 값은 lower bound 가 아니라 safety net.
const POLL_FALLBACK: Duration = Duration::from_secs(2);

struct TailState {
    file: File,
    pos: u64,
    /// 다음 read 까지 보존되는 부분 라인 (\n 안 만난 trailing bytes).
    buf: Vec<u8>,
    #[cfg(unix)]
    inode: u64,
}

impl TailState {
    /// 파일을 열고 끝부분 (현재 size) 으로 seek — adapter 시작 시 옛 라인 무시용.
    async fn open_at_end(path: &Path) -> Option<Self> {
        let meta = tokio::fs::metadata(path).await.ok()?;
        let size = meta.len();
        let mut file = OpenOptions::new().read(true).open(path).await.ok()?;
        file.seek(SeekFrom::Start(size)).await.ok()?;
        Some(Self {
            file,
            pos: size,
            buf: Vec::with_capacity(8192),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                meta.ino()
            },
        })
    }

    /// 파일을 처음부터 — 회전 직후 새 latest.log 가 작은 상태에서 호출.
    async fn open_at_start(path: &Path) -> Option<Self> {
        #[cfg(unix)]
        let meta = tokio::fs::metadata(path).await.ok()?;
        let file = OpenOptions::new().read(true).open(path).await.ok()?;
        Some(Self {
            file,
            pos: 0,
            buf: Vec::with_capacity(8192),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                meta.ino()
            },
        })
    }

    /// 파일에서 가능한 만큼 읽어 라인 단위로 parse.
    /// regular file 의 read 는 EOF 에 즉시 Ok(0) 반환하므로 이 함수는 곧 종료.
    async fn read_and_parse(&mut self, out: &mpsc::Sender<ContainerEvent>) -> std::io::Result<()> {
        let mut chunk = [0u8; 8192];
        loop {
            let n = self.file.read(&mut chunk).await?;
            if n == 0 {
                return Ok(());
            }
            self.pos += n as u64;
            self.buf.extend_from_slice(&chunk[..n]);
            self.drain_lines(out).await?;
        }
    }

    async fn drain_lines(&mut self, out: &mpsc::Sender<ContainerEvent>) -> std::io::Result<()> {
        loop {
            let Some(nl) = self.buf.iter().position(|&b| b == b'\n') else {
                return Ok(());
            };
            let line_bytes: Vec<u8> = self.buf.drain(..=nl).collect();
            let line_str = String::from_utf8_lossy(&line_bytes);
            let line = line_str.trim_end_matches(['\n', '\r']);
            if let Some(event) = parse_line(line) {
                tracing::debug!(?event, "플레이어 이벤트");
                if out.send(ContainerEvent::Player(event)).await.is_err() {
                    // receiver dropped — 상위가 종료 중. 조용히 빠짐.
                    return Ok(());
                }
            }
        }
    }
}

/// `log_dir/latest.log` 를 영구적으로 watch 한다. 정상 흐름에서는 무한 루프.
/// 디렉토리 watch 가 불가능한 (path 부재 등) 경우만 Err.
pub async fn watch_logs(log_dir: PathBuf, out: mpsc::Sender<ContainerEvent>) -> Result<()> {
    let target = log_dir.join(FILE_NAME);
    tracing::info!(target = %target.display(), "log_tail watch 시작");

    // notify 의 sync callback → tokio mpsc bridge.
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<()>();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: notify::Result<Event>| {
            if res.is_ok() {
                let _ = notify_tx.send(());
            }
        })
        .context("notify watcher 생성 실패")?;

    watcher
        .watch(&log_dir, RecursiveMode::NonRecursive)
        .with_context(|| format!("디렉토리 watch 실패: {}", log_dir.display()))?;

    // 시작 시점에 이미 latest.log 가 있으면 끝부분에서 follow (옛 라인 무시).
    let mut tail: Option<TailState> = TailState::open_at_end(&target).await;
    if tail.is_some() {
        tracing::info!("기존 latest.log 끝부분에서 follow");
    } else {
        tracing::info!("latest.log 부재 — 등장 대기");
    }

    loop {
        // pending events 모두 drain — 여러 modify 가 batched 됐을 수 있으니 한 번에 처리.
        while notify_rx.try_recv().is_ok() {}

        let cur_meta = tokio::fs::metadata(&target).await.ok();
        match (tail.as_ref(), cur_meta.as_ref()) {
            (None, Some(_)) => {
                tracing::info!("latest.log 등장 → open at start");
                tail = TailState::open_at_start(&target).await;
            }
            (Some(t), Some(m)) => {
                let cur_size = m.len();
                let inode_changed = {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        m.ino() != t.inode
                    }
                    #[cfg(not(unix))]
                    {
                        false
                    }
                };
                if inode_changed {
                    tracing::info!("inode 변화 → 회전 감지, reopen at start");
                    tail = TailState::open_at_start(&target).await;
                } else if cur_size < t.pos {
                    tracing::info!("size 축소 → truncate 감지, reopen at start");
                    tail = TailState::open_at_start(&target).await;
                }
            }
            (Some(_), None) => {
                // 옛 inode 의 fd 는 아직 유효 (rename 만 됐을 수 있음). 곧 새 latest.log 가
                // 만들어지면 위의 inode_changed branch 에서 reopen.
                tracing::debug!("latest.log path 없음 — fd 유지하며 새 파일 대기");
            }
            (None, None) => {
                // 둘 다 없음 — 다음 event 까지 대기.
            }
        }

        if let Some(t) = tail.as_mut() {
            if let Err(e) = t.read_and_parse(&out).await {
                tracing::warn!(error = %e, "tail read 에러 → reopen at end");
                tail = TailState::open_at_end(&target).await;
            }
        }

        tokio::select! {
            _ = notify_rx.recv() => {}
            _ = tokio::time::sleep(POLL_FALLBACK) => {}
        }
    }
}

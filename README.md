# pengport-adapter-minecraft

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](https://www.gnu.org/licenses/agpl-3.0)

[PengPort](https://github.com/LostCHoco/PengPort) PSP (PengPort Service Protocol) 의 **Minecraft 카테고리 어댑터**. Docker Engine API 로 Minecraft 컨테이너의 로그를 tail 하고 RCON 으로 접속자 목록을 동기화하여, PSP 표준 endpoints (`manifest` / `status` / `events`) 응답을 노출합니다.

본가 PengPort 클라이언트는 이 어댑터의 manifest 만 받아 generic ServiceCard 로 렌더링 — Minecraft 카테고리 종속 코드는 본가 0줄, 모두 이 저장소에 있음.

## PSP endpoints

| 경로 | 메서드 | 응답 |
|---|---|---|
| `/.well-known/pengport-service` | GET | `ServiceManifest` JSON (action: `native_third_party_app` + prism-launcher config) |
| `/pengport/status` | GET | `StatusResponse` JSON (online + players metric) |
| `/pengport/events` | GET (SSE) | `ServiceEvent` 스트림 (`status_changed`, `notification`) |

## 환경변수

### 필수

| 변수 | 의미 |
|---|---|
| `MC_ID` | service id (catalog `[[services]] id` 와 일치) |
| `MC_NAME` | manifest name (사용자 표시 이름) |
| `MC_CONTAINER` | Docker 컨테이너 이름 (logs tail 대상) |
| `RCON_ADDRESS` | `host:port` (컨테이너 네트워크 내) |
| `RCON_PASSWORD` | RCON 비밀번호 |
| `MC_HOST` | 클라이언트가 접속할 도메인/IP (public) |
| `MC_PORT` | 클라이언트가 접속할 포트 |
| `MC_VERSION` | Minecraft 버전 (예: `1.21.1`) |
| `MC_LOADER` | `vanilla` / `fabric` / `forge` / `neoforge` / `quilt` |

### 선택

| 변수 | 기본값 / 의미 |
|---|---|
| `BIND` | HTTP 리슨 (default `0.0.0.0:8080`) |
| `MC_LOADER_VERSION` | vanilla 외에는 필수 |
| `PACKWIZ_URL` | packwiz pack.toml URL (모드팩 자동 동기화) |
| `MC_DISPLAY_NAME` | Prism 인스턴스 표시 이름 (default = MC_NAME) |
| `MC_JAVA_MAJOR` | Java major (예: 21) |
| `MC_DESCRIPTION` | manifest description |
| `MC_ICON_URL` | manifest icon |
| `EVENTS_TOKEN` | events SSE 인증 토큰 (없으면 인증 없음) |
| `PSP_PUBLIC_BASE_URL` | manifest 의 endpoints URL prefix (publish 도메인) |

## 빠른 시작

```bash
docker run -d \
  --name adapter-modded \
  -p 8080:8080 \
  -v /var/run/docker.sock:/var/run/docker.sock:ro \
  -e MC_ID=modded-mc \
  -e MC_NAME="알파펭" \
  -e MC_CONTAINER=ms-mode \
  -e RCON_ADDRESS=ms-mode:25575 \
  -e RCON_PASSWORD="${RCON_MODDED_PASSWORD}" \
  -e MC_HOST=play.example.com \
  -e MC_PORT=25566 \
  -e MC_VERSION=1.21.1 \
  -e MC_LOADER=fabric \
  -e MC_LOADER_VERSION=0.18.4 \
  -e MC_JAVA_MAJOR=21 \
  -e PACKWIZ_URL="https://cdn.example.com/modded/pack.toml" \
  -e PSP_PUBLIC_BASE_URL="https://play.example.com/services/modded-mc" \
  -e EVENTS_TOKEN="${EVENTS_TOKEN}" \
  --network modded_default \
  ghcr.io/lostchoco/pengport-adapter-minecraft:latest
```

## 보안 권장

- **Docker socket 마운트**: 어댑터가 logs tail 을 위해 `/var/run/docker.sock:ro` 가 필요합니다. 운영 환경에서는 [docker-socket-proxy](https://github.com/Tecnativa/docker-socket-proxy) 로 `containers/<name>/logs` API 만 화이트리스트 권장.
- **RCON 비밀번호 회전**: Minecraft RCON 은 권한 분리가 없어 `op` 권한과 동일. 어댑터 전용 read-only 계정이 없으니 `EVENTS_TOKEN` 과 분리해 보관 + 정기 회전.
- **EVENTS_TOKEN 비교**: 어댑터는 `subtle::ConstantTimeEq` 로 timing-safe 비교.

## 빌드

워크스페이스 멤버일 때:
```bash
cargo build -p pengport-adapter-minecraft --release
```

독립 저장소일 때:
```bash
cargo build --release
```

Docker:
```bash
docker build -t pengport-adapter-minecraft .
```

## 라이선스

[AGPL-3.0-only](LICENSE). PengPort 본가와 동일.

## 새 카테고리 어댑터 추가하기

PSP 정신: 카테고리 종속 코드는 본가에 두지 않고 별도 어댑터로. 다음과 같은 패턴이 표준입니다:

1. 새 어댑터 저장소 (`pengport-adapter-<category>`) 생성
2. PSP endpoints 3개 (`manifest` / `status` / `events`) 응답
3. manifest 의 actions[].kind 는 본가의 표준 native action kinds 중 하나만 사용 (open_url / open_protocol / submit_form / native_third_party_app)
4. 운영자가 docker-compose 에 컨테이너 추가 + `services.d/<id>.toml` 등록

명세: [`docs/spec/psp-v1.md`](https://github.com/LostCHoco/PengPort/blob/master/docs/spec/psp-v1.md).

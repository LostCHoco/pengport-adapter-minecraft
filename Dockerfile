# syntax=docker/dockerfile:1.7
# pengport-adapter-minecraft standalone 멀티스테이지 빌드.
# shared 는 git dependency 로 cargo 가 자동 fetch.

FROM rust:1-bookworm AS builder

WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends git && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml ./
COPY src src

RUN cargo build --release \
 && strip target/release/adapter-minecraft


FROM debian:bookworm-slim AS runtime

# docker-cli 필요 — adapter 가 `docker logs -f` subprocess 로 컨테이너 로그를 follow.
# bollard 의 logs stream 이 follow 모드에서 매 1~30초 즉시 종료되는 quirk 회피용.
# /var/run/docker.sock 은 runtime 시 host 에서 read-only mount.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates docker.io \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/adapter-minecraft /usr/local/bin/adapter-minecraft

ENV RUST_LOG=info
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/adapter-minecraft"]

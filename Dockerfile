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

# adapter 는 minecraft 의 logs/latest.log 를 host bind mount 로 직접 inotify watch.
# docker engine 호출 (logs follow / events 등) 안 하므로 docker CLI 의존 없음.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/adapter-minecraft /usr/local/bin/adapter-minecraft

ENV RUST_LOG=info
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/adapter-minecraft"]

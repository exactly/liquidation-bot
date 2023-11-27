# syntax=docker/dockerfile:1.2
FROM rustlang/rust:nightly-bookworm-slim AS builder

# hadolint ignore=DL3008
RUN apt-get update && apt-get install --no-install-recommends -y pkg-config libssl-dev

WORKDIR /liq-bot

COPY node_modules node_modules
COPY Cargo.lock .
COPY Cargo.toml .
COPY deployments deployments
COPY src src
COPY lib lib

RUN rustup component add rustfmt clippy

RUN cargo fmt --check \
 && cargo clippy -- -D warnings \
 && cargo build --release

FROM debian:bookworm-slim

# hadolint ignore=DL3008
RUN apt-get update && apt-get install --no-install-recommends -y ca-certificates \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /liq-bot

COPY --from=builder /liq-bot/target/release/liq-bot .
COPY --from=builder /liq-bot/deployments deployments
COPY --from=builder /liq-bot/node_modules/@exactly/protocol/deployments node_modules/@exactly/protocol/deployments

ENTRYPOINT [ "/liq-bot/liq-bot" ]

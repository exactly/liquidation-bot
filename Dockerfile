# syntax=docker/dockerfile:1.2
FROM rust:slim-bullseye AS builder

RUN apt-get update && apt-get -y --no-install-recommends install pkg-config libssl-dev

WORKDIR /liq-bot

COPY node_modules node_modules
COPY Cargo.lock .
COPY Cargo.toml .
COPY deployments deployments
COPY src src
COPY lib lib

RUN cargo build --release

FROM debian:stable-slim

WORKDIR /liq-bot

COPY --from=builder /liq-bot/target/release/liq-bot .
COPY --from=builder /liq-bot/deployments deployments
COPY --from=builder /liq-bot/node_modules/@exactly-protocol/protocol/deployments node_modules/@exactly-protocol/protocol/deployments

ENTRYPOINT [ "/liq-bot/liq-bot" ]

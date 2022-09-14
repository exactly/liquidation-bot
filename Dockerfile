# syntax=docker/dockerfile:1.2
FROM rust:slim-bullseye AS builder

WORKDIR /liq-bot

COPY node_modules node_modules
COPY Cargo.lock .
COPY Cargo.toml .
COPY deployments deployments
COPY src src

RUN cargo build --release

FROM debian:stable-slim

WORKDIR /liq-bot

COPY --from=builder /liq-bot/target/release/liq-bot .
COPY --from=builder /liq-bot/deployments deployments
COPY --from=builder /liq-bot/node_modules/@exactly-protocol/protocol/deployments node_modules/@exactly-protocol/protocol/deployments

ENTRYPOINT [ "/liq-bot/liq-bot" ]

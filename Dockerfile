# syntax=docker/dockerfile:1.2
FROM rust

WORKDIR /

COPY Cargo.toml .
COPY Cargo.lock .
COPY src src
COPY node_modules node_modules

RUN cargo build

ARG CHAIN_ID
ARG PRIVATE_KEY
ARG ETH_RINKEBY_PROVIDER

ENV CHAIN_ID $CHAIN_ID
ENV PRIVATE_KEY $PRIVATE_KEY
ENV ETH_RINKEBY_PROVIDER $ETH_RINKEBY_PROVIDER

ENTRYPOINT [ "target/debug/liq-rs" ]

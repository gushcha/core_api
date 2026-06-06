FROM rust:1.87-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y \
    cmake \
    build-essential \
    libssl-dev \
    libcurl4-openssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./

RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

COPY src ./src

RUN touch src/main.rs && cargo build --release

FROM ubuntu:24.04

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/core_api /usr/local/bin/core_api

EXPOSE 3000

CMD ["core_api"]

FROM rust:1.87-slim AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./

# Cache dependencies by building a dummy binary first
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release && \
    rm -rf src

COPY src ./src

# Touch main.rs so cargo rebuilds the real binary
RUN touch src/main.rs && cargo build --release

FROM ubuntu:24.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/core_api /usr/local/bin/core_api

EXPOSE 3000

CMD ["core_api"]

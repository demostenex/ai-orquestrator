# Stage 1: builder
FROM rust:1.82-alpine3.20 AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

# Stage 2: runtime
FROM alpine:3.20
RUN apk add --no-cache git ca-certificates
COPY --from=builder /build/target/release/ai-orchestrator /usr/local/bin/ai-orchestrator
WORKDIR /workspace
ENTRYPOINT ["ai-orchestrator"]

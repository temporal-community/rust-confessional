FROM rust:1.88-bookworm AS build-base

RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY static ./static

FROM build-base AS test

RUN --mount=type=cache,id=rust-confessional-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rust-confessional-target,target=/build/target \
    cargo test --locked

FROM build-base AS lint

RUN --mount=type=cache,id=rust-confessional-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rust-confessional-target,target=/build/target \
    cargo fmt --all -- --check \
    && cargo clippy --locked --all-targets -- -D warnings

FROM build-base AS builder

RUN --mount=type=cache,id=rust-confessional-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=rust-confessional-target,target=/build/target \
    cargo build --locked --release \
    && mkdir -p /out \
    && cp /build/target/release/naive /build/target/release/stage /build/target/release/worker /out/

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 app

WORKDIR /app
COPY --from=builder /out/stage /usr/local/bin/stage
COPY --from=builder /out/worker /usr/local/bin/worker
COPY --from=builder /out/naive /usr/local/bin/naive
COPY static ./static
RUN mkdir -p /app/data && chown -R app:app /app

USER app
EXPOSE 3000

CMD ["stage"]

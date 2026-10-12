# ── Aşama 1: Build ────────────────────────────────────────────────────────────
FROM rust:1.85-slim AS builder

# aws-lc-sys (rustls bağımlılığı) için: cmake + clang + perl
# sqlx runtime-tokio-rustls ve axum-server tls-rustls bu bağımlılıkları getirir
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake clang perl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Bağımlılıkları önbelleğe al (sadece Cargo manifests)
COPY Cargo.toml Cargo.lock ./
COPY crates/nova-dex/Cargo.toml        crates/nova-dex/Cargo.toml
COPY crates/nova-dex-server/Cargo.toml crates/nova-dex-server/Cargo.toml

# Dummy kaynaklar — bağımlılık katmanını ayır (pahalı C bağımlılıkları bir kez derlenir)
RUN mkdir -p crates/nova-dex/src crates/nova-dex-server/src \
    && echo "fn main() {}" > crates/nova-dex-server/src/main.rs \
    && echo "" > crates/nova-dex/src/lib.rs \
    && cargo build --release -p nova-dex-server \
    && rm -rf crates/nova-dex/src crates/nova-dex-server/src

# Gerçek kaynakları kopyala ve derle
COPY crates/ crates/
RUN touch crates/nova-dex/src/lib.rs crates/nova-dex-server/src/main.rs \
    && cargo build --release -p nova-dex-server

# ── Aşama 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 wget \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1001 nova \
    && useradd --uid 1001 --gid nova --shell /bin/sh --create-home nova

COPY --from=builder /build/target/release/nova-dex-server /usr/local/bin/nova-dex-server

USER nova
WORKDIR /home/nova

ENV PORT=3000

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=5 \
    CMD wget -qO- http://localhost:3000/health || exit 1

ENTRYPOINT ["/usr/local/bin/nova-dex-server"]

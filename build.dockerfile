FROM rust:1.98.1-slim-trixie

RUN useradd -m -d /builder -s /bin/bash builder
USER builder

WORKDIR /builder
COPY --chown=builder:builder Cargo.toml Cargo.lock ./
RUN mkdir src &&\
    echo "fn main() {}" > src/main.rs &&\
    cargo fetch --locked &&\
    cargo build --release --locked

COPY --chown=builder:builder src src/
RUN touch src/main.rs &&\
    cargo test --locked &&\
    cargo build --release --locked

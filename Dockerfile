# build openapi-codegen
FROM docker.io/lukemathwalker/cargo-chef:latest-rust-1.85 AS chef
WORKDIR /app

FROM chef AS planner

COPY Cargo.toml .
COPY Cargo.lock .
COPY build.rs .
COPY src /app/src

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS openapi-codegen-builder

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml .
COPY Cargo.lock .
COPY build.rs .
COPY src /app/src

RUN cargo build --release --bin openapi-codegen

# build goimports
FROM docker.io/golang:1.24-bookworm AS goimports-builder
RUN go install golang.org/x/tools/cmd/goimports@latest

# build rubyfmt
FROM docker.io/rust:1.85 AS rubyfmt-builder
WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends ruby bison && \
    apt-get clean

RUN git clone https://github.com/fables-tales/rubyfmt.git \
    --recurse-submodules --shallow-submodules /app && \
    git checkout 71cbb4adc53d3d8b36a6f1b3dcff87865d0204b8

RUN cargo build --release

# main container
FROM docker.io/ubuntu:noble

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends curl default-jre-headless && \
    apt-get clean

# Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y \
    --profile minimal \
    --no-modify-path \
    --no-update-default-toolchain \
    --default-toolchain nightly-2025-02-27 \
    --component rustfmt

ENV PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/root/.cargo/bin/"

# Javascript
COPY --from=ghcr.io/biomejs/biome:1.9.4 /usr/local/bin/biome /usr/bin/biome

# Python
COPY --from=ghcr.io/astral-sh/ruff:0.9.8 /ruff /usr/bin/ruff

# Java
RUN echo "25157797a0a972c2290b5bc71530c4f7ad646458025e3484412a6e5a9b8c9aa6 google-java-format-1.25.2-all-deps.jar" > google-java-format-1.25.2-all-deps.jar.sha256 && \
    curl -fsSL --output google-java-format-1.25.2-all-deps.jar "https://github.com/google/google-java-format/releases/download/v1.25.2/google-java-format-1.25.2-all-deps.jar" && \
    sha256sum google-java-format-1.25.2-all-deps.jar.sha256 -c && \
    rm google-java-format-1.25.2-all-deps.jar.sha256 && \
    mv google-java-format-1.25.2-all-deps.jar /usr/bin/  && \
    echo '#!/usr/bin/bash\njava -jar /usr/bin/google-java-format-1.25.2-all-deps.jar $@' > /usr/bin/google-java-format  && \
    chmod +x /usr/bin/google-java-format

# Kotlin
RUN echo "5e7eb28a0b2006d1cefbc9213bfc73a8191ec2f85d639ec4fc4ec0cd04212e82 ktfmt-0.54-jar-with-dependencies.jar" > ktfmt-0.54-jar-with-dependencies.jar.sha256  && \
    curl -fsSL --output ktfmt-0.54-jar-with-dependencies.jar "https://github.com/facebook/ktfmt/releases/download/v0.54/ktfmt-0.54-jar-with-dependencies.jar"  && \
    sha256sum ktfmt-0.54-jar-with-dependencies.jar.sha256 -c  && \
    rm ktfmt-0.54-jar-with-dependencies.jar.sha256  && \
    mv ktfmt-0.54-jar-with-dependencies.jar /usr/bin/  && \
    echo '#!/usr/bin/bash\njava -jar /usr/bin/ktfmt-0.54-jar-with-dependencies.jar $@' > /usr/bin/ktfmt  && \
    chmod +x /usr/bin/ktfmt

# Go
COPY --from=goimports-builder /go/bin/goimports /usr/bin
COPY --from=goimports-builder /usr/local/go/bin/gofmt /usr/bin

# openapi-codegen
COPY --from=openapi-codegen-builder /app/target/release/openapi-codegen /usr/bin/

# Ruby
COPY --from=rubyfmt-builder /app/target/release/rubyfmt-main /usr/bin/rubyfmt


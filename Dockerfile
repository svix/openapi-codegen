# build openapi-codegen
FROM docker.io/rust:1.97.1-slim-trixie AS chef
RUN cargo install --locked cargo-chef@0.1.77
WORKDIR /app
WORKDIR /app

FROM chef AS planner

COPY Cargo.toml .
COPY Cargo.lock .
COPY src /app/src

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS openapi-codegen-builder

COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml .
COPY Cargo.lock .
COPY src /app/src

RUN cargo build --release --bin openapi-codegen

FROM alpine:3.24 AS downloader
SHELL ["/bin/sh", "-eu", "-c"]
COPY --chown=root:root --chmod=755 download-tool.sh /usr/local/bin/download-tool.sh
RUN --mount=target=/var/cache/apk,type=cache,sharing=locked apk add curl binutils coreutils

# download rubyfmt
FROM downloader AS rubyfmt-downloader
RUN download-tool.sh rubyfmt

# build csharpier
FROM downloader AS csharpier-builder
RUN --mount=target=/var/cache/apk,type=cache,sharing=locked apk add git dotnet10-sdk

RUN <<EOF
    git clone --depth 1 --branch=1.2.6 https://github.com/belav/csharpier /app
    cd /app || exit 1
    case "$(uname -m)" in
        aarch64)
            DOTNET_PLATFORM=linux-musl-arm64
            ;;
        x86_64)
            DOTNET_PLATFORM=linux-musl-amd64
            ;;
        *)
            echo >&2 "unhandled platform $(uname -m)"
            exit 1
            ;;
    esac
    dotnet publish --framework net9.0 -o output \
        -r ${DOTNET_PLATFORM} /p:StripSymbols=true \
        /p:InvariantGlobalization=true /p:SelfContained=true \
        /p:PublishSingleFile=true Src/CSharpier.Cli
EOF


# build goimports
FROM docker.io/golang:1.25-alpine AS goimports-builder

SHELL ["/bin/sh", "-eu", "-c"]
RUN <<EOF
    go install golang.org/x/tools/cmd/goimports@latest
    go install github.com/segmentio/golines@v0.13.0
    go install mvdan.cc/gofumpt@v0.9.1

    # will copy /usr/local/go into release image later, trims about 170mb
    rm -rf /usr/local/go/*.md
    rm -rf /usr/local/go/api
    rm -rf /usr/local/go/doc
    rm -rf /usr/local/go/pkg/tool/**/*
    rm -rf /usr/local/go/src/*
    rm -rf /usr/local/go/test/*
EOF

# download java formatter
FROM downloader AS javafmt-downloader
RUN download-tool.sh javafmt

# Javascript
FROM downloader AS biome-downloader
RUN download-tool.sh biome

# main image
FROM alpine:3.24 AS main
SHELL ["/bin/sh", "-eu", "-c"]
ENV PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/go/bin:/root/.cargo/bin"
RUN --mount=target=/var/cache/apk,type=cache,sharing=locked apk add openjdk17-jre-headless curl gcompat libgcc ruff libstdc++

# Kotlin
RUN <<EOF
    echo "5e7eb28a0b2006d1cefbc9213bfc73a8191ec2f85d639ec4fc4ec0cd04212e82 ktfmt-0.54-jar-with-dependencies.jar" > ktfmt-0.54-jar-with-dependencies.jar.sha256
    curl -fsSL --output ktfmt-0.54-jar-with-dependencies.jar "https://github.com/facebook/ktfmt/releases/download/v0.54/ktfmt-0.54-jar-with-dependencies.jar"
    sha256sum ktfmt-0.54-jar-with-dependencies.jar.sha256 -c
    rm ktfmt-0.54-jar-with-dependencies.jar.sha256
    mv ktfmt-0.54-jar-with-dependencies.jar /usr/bin/
    echo "#!/bin/sh" >> /usr/bin/ktfmt
    echo 'exec /usr/bin/java  -jar /usr/bin/ktfmt-0.54-jar-with-dependencies.jar $@' >> /usr/bin/ktfmt
    chmod +x /usr/bin/ktfmt
EOF

# Java
RUN <<EOF
    echo "25157797a0a972c2290b5bc71530c4f7ad646458025e3484412a6e5a9b8c9aa6 google-java-format-1.25.2-all-deps.jar" > google-java-format-1.25.2-all-deps.jar.sha256
    curl -fsSL --output google-java-format-1.25.2-all-deps.jar "https://github.com/google/google-java-format/releases/download/v1.25.2/google-java-format-1.25.2-all-deps.jar"
    sha256sum google-java-format-1.25.2-all-deps.jar.sha256 -c
    rm google-java-format-1.25.2-all-deps.jar.sha256
    mv google-java-format-1.25.2-all-deps.jar /usr/bin/
    echo "#!/bin/sh" >> /usr/bin/google-java-format
    echo 'exec /usr/bin/java -jar /usr/bin/google-java-format-1.25.2-all-deps.jar $@' >> /usr/bin/google-java-format
    chmod +x /usr/bin/google-java-format
EOF


# Javascript
COPY --chown=root:root --chmod=755 --from=biome-downloader /usr/bin/biome /usr/bin/biome

# Ruby
COPY --chown=root:root --chmod=755 --from=rubyfmt-downloader /tmp/rubyfmt /usr/bin/rubyfmt

# Go
COPY --from=goimports-builder /usr/local/go/ /usr/local/go/
COPY --chown=root:root --chmod=755 --from=goimports-builder /go/bin/goimports /usr/bin
COPY --chown=root:root --chmod=755 --from=goimports-builder /go/bin/golines /usr/bin
COPY --chown=root:root --chmod=755 --from=goimports-builder /go/bin/gofumpt /usr/bin

# C#
COPY --chown=root:root --chmod=755 --from=csharpier-builder /app/output/CSharpier /usr/bin/csharpier

# Rust
# All of this craziness reduces the image size by about 600Mb
RUN --mount=target=/var/cache/apk,type=cache,sharing=locked <<EOF
    apk add binutils
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y \
    --profile minimal \
    --no-modify-path \
    --no-update-default-toolchain \
    --default-toolchain nightly-2025-02-27 \
    --component rustfmt
    rm -rf /root/.rustup/toolchains/nightly-*/lib/rustlib
    rm /root/.rustup/toolchains/nightly-*/bin/cargo*
    rm /root/.rustup/toolchains/nightly-*/bin/rust-*
    rm /root/.rustup/toolchains/nightly-*/bin/rustc
    rm /root/.rustup/toolchains/nightly-*/bin/rustdoc
    rm -rf /root/.rustup/toolchains/nightly-*/share
    strip /root/.rustup/toolchains/nightly-*/lib/librustc_driver-*.so
    apk del binutils
EOF

# PHP
RUN --mount=target=/var/cache/apk,type=cache,sharing=locked apk add php84-tokenizer php84-phar php84-iconv php84-mbstring php84
RUN <<EOF
    echo "0a9ad9fd8996064ff9aabfba3cb1cea148e3a1785263f6f91ff1431def402513  php-cs-fixer.phar" >  php-cs-fixer.phar.sha256
    curl -fsSL --output php-cs-fixer.phar https://github.com/PHP-CS-Fixer/PHP-CS-Fixer/releases/download/v3.86.0/php-cs-fixer.phar
    sha256sum -c php-cs-fixer.phar.sha256
    rm php-cs-fixer.phar.sha256
    mv php-cs-fixer.phar /usr/share
EOF

# Java formatter for code samples
COPY --from=javafmt-downloader /usr/bin/palantir-java-format.bin /usr/bin/palantir-java-format.bin

# openapi-codegen
COPY --from=openapi-codegen-builder /app/target/release/openapi-codegen /usr/bin/

LABEL org.opencontainers.image.authors="support@svix.com" \
      org.opencontainers.image.description="Svix's OpenAPI code generation" \
      org.opencontainers.image.title="openapi-codegen" \
      org.opencontainers.image.vendor="Svix" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.base.name="docker.io/alpine:3.24"

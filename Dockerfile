# build openapi-codegen
FROM docker.io/lukemathwalker/cargo-chef:latest-rust-1.88 AS chef
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


# download rubyfmt
FROM alpine:3.21 AS rubyfmt-downloader
ARG RUBYFMT_DL_LINK="https://github.com/fables-tales/rubyfmt/releases/download/v0.11.67-0/rubyfmt-v0.11.67-0-Linux-x86_64.tar.gz"
ARG RUBYFMT_SHA256="40f734a83edcc5f03f789606293af9ea622ea2a4fc3091c551b7c1f817087dcd"
RUN apk add --no-cache curl binutils
RUN echo "${RUBYFMT_SHA256} rubyfmt.tar.gz" > rubyfmt.tar.gz.sha256 && \
    curl -fsSL --output rubyfmt.tar.gz "${RUBYFMT_DL_LINK}" && \
    sha256sum rubyfmt.tar.gz.sha256 -c && \
    tar xfv rubyfmt.tar.gz && \
    strip tmp/releases/v0.11.67-0-Linux/rubyfmt


# build csharpier
FROM alpine:3.21 AS csharpier-builder
ARG DOTNET_PLATFORM="linux-musl-amd64"
WORKDIR /app
RUN apk add --no-cache git dotnet9-sdk

# this is csharpier v1.0.1
RUN git clone https://github.com/belav/csharpier /app && \
    git checkout 488679295cf50e84e6cac15f308d7e5a362c245c

RUN dotnet publish --framework net9.0 -o output \
    -r ${DOTNET_PLATFORM} /p:StripSymbols=true \
    /p:InvariantGlobalization=true /p:SelfContained=true \
    /p:PublishSingleFile=true Src/CSharpier.Cli

# build goimports
FROM docker.io/golang:1.24-alpine AS goimports-builder
RUN go install golang.org/x/tools/cmd/goimports@latest
# will copy /usr/local/go into release image later, trims about 170mb
RUN rm -rf /usr/local/go/*.md && \
    rm -rf /usr/local/go/api && \
    rm -rf /usr/local/go/doc && \
    rm -rf /usr/local/go/pkg/tool/**/* && \
    rm -rf /usr/local/go/src/* && \
    rm -rf /usr/local/go/test/*


# main image
FROM alpine:3.21
ENV PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/local/go/bin:/root/.cargo/bin"
RUN apk add --no-cache openjdk17-jre-headless curl gcompat libgcc ruff libstdc++ php84-tokenizer php84-phar php84 

# Kotlin
RUN echo "5e7eb28a0b2006d1cefbc9213bfc73a8191ec2f85d639ec4fc4ec0cd04212e82 ktfmt-0.54-jar-with-dependencies.jar" > ktfmt-0.54-jar-with-dependencies.jar.sha256  && \
    curl -fsSL --output ktfmt-0.54-jar-with-dependencies.jar "https://github.com/facebook/ktfmt/releases/download/v0.54/ktfmt-0.54-jar-with-dependencies.jar"  && \
    sha256sum ktfmt-0.54-jar-with-dependencies.jar.sha256 -c  && \
    rm ktfmt-0.54-jar-with-dependencies.jar.sha256  && \
    mv ktfmt-0.54-jar-with-dependencies.jar /usr/bin/  && \
    echo "#!/bin/sh" >> /usr/bin/ktfmt && \
    echo '/usr/bin/java  -jar /usr/bin/ktfmt-0.54-jar-with-dependencies.jar $@' >> /usr/bin/ktfmt && \
    chmod +x /usr/bin/ktfmt

# Java
RUN echo "25157797a0a972c2290b5bc71530c4f7ad646458025e3484412a6e5a9b8c9aa6 google-java-format-1.25.2-all-deps.jar" > google-java-format-1.25.2-all-deps.jar.sha256 && \
    curl -fsSL --output google-java-format-1.25.2-all-deps.jar "https://github.com/google/google-java-format/releases/download/v1.25.2/google-java-format-1.25.2-all-deps.jar" && \
    sha256sum google-java-format-1.25.2-all-deps.jar.sha256 -c && \
    rm google-java-format-1.25.2-all-deps.jar.sha256 && \
    mv google-java-format-1.25.2-all-deps.jar /usr/bin/  && \
    echo "#!/bin/sh" >> /usr/bin/google-java-format && \
    echo '/usr/bin/java -jar /usr/bin/google-java-format-1.25.2-all-deps.jar $@' >> /usr/bin/google-java-format && \
    chmod +x /usr/bin/google-java-format


# Javascript
ARG BIOME_DL_LINK="https://github.com/biomejs/biome/releases/download/%40biomejs%2Fbiome%402.1.4/biome-linux-x64-musl"
ARG BIOME_SHA256="6d6bd2213cffab0d68d741c0be466bcd21cd6f5eca1e0e5aac2a991bf9f17cf2"
RUN echo "${BIOME_SHA256} biome" > biome.sha256 && \
    curl -fsSL --output biome "${BIOME_DL_LINK}" && \
    sha256sum biome.sha256 -c && \
    rm biome.sha256 && \
    mv biome /usr/bin/  && \
    chmod +x /usr/bin/biome

# Ruby
COPY --from=rubyfmt-downloader /tmp/releases/v0.11.67-0-Linux/rubyfmt /usr/bin/rubyfmt

# Go
COPY --from=goimports-builder /usr/local/go/ /usr/local/go/
COPY --from=goimports-builder /go/bin/goimports /usr/bin

# C#
COPY --from=csharpier-builder /app/output/CSharpier /usr/bin/csharpier

# PHP
RUN echo "0a9ad9fd8996064ff9aabfba3cb1cea148e3a1785263f6f91ff1431def402513  php-cs-fixer.phar" >  php-cs-fixer.phar.sha256  && \
    curl -fsSL --output php-cs-fixer.phar https://github.com/PHP-CS-Fixer/PHP-CS-Fixer/releases/download/v3.86.0/php-cs-fixer.phar && \
    sha256sum -c php-cs-fixer.phar.sha256 && \
    rm php-cs-fixer.phar.sha256 && \
    mv php-cs-fixer.phar /usr/share

# Rust
# All of this craziness reduces the image size by about 600Mb
RUN apk add --no-cache binutils && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y \
    --profile minimal \
    --no-modify-path \
    --no-update-default-toolchain \
    --default-toolchain nightly-2025-02-27 \
    --component rustfmt && \
    rm -rf /root/.rustup/toolchains/nightly-*/lib/rustlib && \
    rm /root/.rustup/toolchains/nightly-*/bin/cargo* && \
    rm /root/.rustup/toolchains/nightly-*/bin/rust-* && \
    rm /root/.rustup/toolchains/nightly-*/bin/rustc && \
    rm /root/.rustup/toolchains/nightly-*/bin/rustdoc && \
    rm -rf /root/.rustup/toolchains/nightly-*/share && \
    strip /root/.rustup/toolchains/nightly-*/lib/librustc_driver-*.so && \
    apk del binutils

# openapi-codegen
COPY --from=openapi-codegen-builder /app/target/release/openapi-codegen /usr/bin/

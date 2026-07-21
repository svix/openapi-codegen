#!/bin/sh

set -eu

ARCH="$(uname -m)"

case "$ARCH" in
    aarch64)
        BIOME_DL_LINK=https://github.com/biomejs/biome/releases/download/%40biomejs%2Fbiome%402.1.4/biome-linux-arm64-musl
        BIOME_SHA256=ffa05ea6ec0e73072e46301a692eb9413d5b683366e86ab7243414ae944f4ec4
        RUBYFMT_DL_LINK=https://github.com/fables-tales/rubyfmt/releases/download/v0.11.67-0/rubyfmt-v0.11.67-0-Linux-aarch64.tar.gz
        RUBYFMT_SHA256=805fec1bf5400513058d8ec2d5cde0b497182b80828957ef0239190aa1f01092
        JAVAFMT_DL_LINK=https://repo1.maven.org/maven2/com/palantir/javaformat/palantir-java-format-native/2.75.0/palantir-java-format-native-2.75.0-nativeImage-linux-glibc_aarch64.bin
        JAVAFMT_SHA256=371e226632a5c455f017fe2ce2a614abe8cf81c743b4c27fb998373b790c2a3b
        ;;
    x86_64)
        BIOME_DL_LINK=https://github.com/biomejs/biome/releases/download/%40biomejs%2Fbiome%402.1.4/biome-linux-x64-musl
        BIOME_SHA256=6d6bd2213cffab0d68d741c0be466bcd21cd6f5eca1e0e5aac2a991bf9f17cf2
        RUBYFMT_DL_LINK=https://github.com/fables-tales/rubyfmt/releases/download/v0.11.67-0/rubyfmt-v0.11.67-0-Linux-x86_64.tar.gz
        RUBYFMT_SHA256=40f734a83edcc5f03f789606293af9ea622ea2a4fc3091c551b7c1f817087dcd
        JAVAFMT_DL_LINK=https://repo1.maven.org/maven2/com/palantir/javaformat/palantir-java-format-native/2.75.0/palantir-java-format-native-2.75.0-nativeImage-linux-glibc_x86-64.bin
        JAVAFMT_SHA256=9d8c9e65cff44bb847d16b4db2ccbd6dacbe32611eaf2587748013eda931cdac
        ;;
    *)
        echo >&2 "Error: Unhandled architecture $ARCH"
        exit 1
        ;;
esac

TOOL="${1:-}"

WD=$(mktemp -d)

on_exit() {
    status=$?
    cd /
    rm -rf "$WD"
    exit $status
}

trap on_exit EXIT INT TERM

cd "$WD" || exit 1

case "$TOOL" in
    "")
        echo >&2 "Usage: $0 (biome|rubyfmt|javafmt)"
        exit 2
        ;;
    rubyfmt)
        echo "${RUBYFMT_SHA256} rubyfmt.tar.gz" >rubyfmt.tar.gz.sha256
        curl -fsSL --output rubyfmt.tar.gz "${RUBYFMT_DL_LINK}"
        sha256sum rubyfmt.tar.gz.sha256 -c
        tar xfv rubyfmt.tar.gz
        strip tmp/releases/v0.11.67-0-Linux/rubyfmt
        mv tmp/releases/v0.11.67-0-Linux/rubyfmt /tmp/rubyfmt
        ;;
    javafmt)
        echo "${JAVAFMT_SHA256} palantir-java-format.bin" >palantir-java-format.bin.sha256
        curl -fsSL --output palantir-java-format.bin "${JAVAFMT_DL_LINK}"
        sha256sum palantir-java-format.bin.sha256 -c
        mv palantir-java-format.bin /usr/bin
        chmod +x /usr/bin/palantir-java-format.bin
        ;;
    biome)
        echo "${BIOME_SHA256} biome" >biome.sha256
        curl -fsSL --output biome "${BIOME_DL_LINK}"
        sha256sum biome.sha256 -c
        rm biome.sha256
        mv biome /usr/bin/
        chmod +x /usr/bin/biome
        ;;
    *)
        echo >&2 "Unrecognized tool $TOOL"
        exit 1
        ;;
esac

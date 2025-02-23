#!/usr/bin/env bash
if [ -z "$1" ]
  then
    echo "No argument supplied"
    exit 1
fi

set -eox pipefail

if [ "$(uname)" == "Darwin" ]; then
  TAR_BIN="gtar"
else
  TAR_BIN="tar"
fi

BIN_NAME="openapi-codegen"
mkdir -p target/distrib

if [ "$(expr substr $(uname -s) 1 10)" == "MINGW64_NT" ]; then
  PKG_FILENAME="$BIN_NAME-$1.zip"
  7z a -tzip \
    "target/distrib/$PKG_FILENAME" \
    README.md \
    LICENSE \
    "./target/$1/release/$BIN_NAME.exe" -w"./target/$1/release"
else
  PKG_FILENAME="$BIN_NAME-$1.tar.xz"
  $TAR_BIN -czf \
      "target/distrib/$PKG_FILENAME" \
      README.md \
      LICENSE \
      --transform="s|target/$1/release/$BIN_NAME|$BIN_NAME|" \
      "target/$1/release/$BIN_NAME"
fi

cd target/distrib
sha256sum $PKG_FILENAME > "$PKG_FILENAME.sha256"


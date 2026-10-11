#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
tools="$PWD/target/macos-tools"
mkdir -p "$tools"
PATH="$tools:$PATH"
export PATH
export MACOSX_DEPLOYMENT_TARGET=11.0

signer="https://github.com/indygreg/apple-platform-rs/releases/download/apple-codesign%2F0.29.0/apple-codesign-0.29.0-x86_64-unknown-linux-musl.tar.gz"
signer_sum="dbe85cedd8ee4217b64e9a0e4c2aef92ab8bcaaa41f20bde99781ff02e600002"
bomutils="c247002e2f49878bab32dd0fcf805810ae0992cb"

if ! command -v o64-clang >/dev/null; then
    echo "Run this inside the image joseluisq/rust-linux-darwin-builder, with the repository as the working folder:" >&2
    echo "  docker run --rm -v \"\$PWD\":/root/src -w /root/src joseluisq/rust-linux-darwin-builder:1.89.0 sh tools/macos_build.sh" >&2
    exit 1
fi

rustup toolchain install stable --profile minimal --no-self-update -t x86_64-apple-darwin -t aarch64-apple-darwin
rustup default stable

for triple in x86_64-apple-darwin aarch64-apple-darwin; do
    cargo build --release --locked -p ps-app --target "$triple"
done

if [ ! -x "$tools/rcodesign" ]; then
    curl -fsSL -o "$tools/rcodesign.tar.gz" "$signer"
    echo "$signer_sum  $tools/rcodesign.tar.gz" | sha256sum -c -
    tar -xzf "$tools/rcodesign.tar.gz" -C "$tools" --strip-components=1 --wildcards '*/rcodesign'
    rm "$tools/rcodesign.tar.gz"
fi

if [ ! -x "$tools/mkbom" ]; then
    rm -rf "$tools/bomutils"
    git clone -q https://github.com/hogliux/bomutils "$tools/bomutils"
    git -C "$tools/bomutils" checkout -q "$bomutils"
    make -s -C "$tools/bomutils"
    cp "$tools/bomutils/build/bin/mkbom" "$tools/bomutils/build/bin/lsbom" "$tools/"
fi

if ! command -v xorriso >/dev/null; then
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends xorriso
fi

python3 tools/package_macos.py "$@"

#!/bin/bash

set -xeuo pipefail

echo "Cross-compiling static binaries for all platforms..."

# Install targets if not already installed
rustup target add x86_64-unknown-linux-musl 2>/dev/null || true
rustup target add x86_64-pc-windows-gnu 2>/dev/null || true
rustup target add i686-pc-windows-gnu 2>/dev/null || true

# Build all targets in parallel - both debug and release
echo "Building debug targets..."
cargo build --target x86_64-unknown-linux-musl &
cargo build --target x86_64-pc-windows-gnu &  
cargo build --target i686-pc-windows-gnu &

echo "Building release targets..."
cargo build --release --target x86_64-unknown-linux-musl &
cargo build --release --target x86_64-pc-windows-gnu &  
cargo build --release --target i686-pc-windows-gnu &

# Wait for all builds to complete
wait

echo ""
echo "Cross-compile completed. Static binaries:"
echo "Debug builds:"
ls -lh target/x86_64-unknown-linux-musl/debug/dat3
ls -lh target/x86_64-pc-windows-gnu/debug/dat3.exe  
ls -lh target/i686-pc-windows-gnu/debug/dat3.exe
echo ""
echo "Release builds:"
ls -lh target/x86_64-unknown-linux-musl/release/dat3
ls -lh target/x86_64-pc-windows-gnu/release/dat3.exe  
ls -lh target/i686-pc-windows-gnu/release/dat3.exe

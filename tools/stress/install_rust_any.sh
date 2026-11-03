#!/bin/sh
# Installs the stable Rust toolchain on macOS or Linux, for this machine's chip (Intel or Apple Silicon Mac, x86_64 or arm64 Linux).
# Run on the target machine: sh install_rust_any.sh. Output goes to $HOME/rust-install.log and ends with RUSTUP_DONE.
set -u
LOG="$HOME/rust-install.log"
{
    OS=$(uname -s)
    ARCH=$(uname -m)
    case "$OS/$ARCH" in
        Darwin/x86_64) TRIPLE=x86_64-apple-darwin ;;
        Darwin/arm64) TRIPLE=aarch64-apple-darwin ;;
        Linux/x86_64) TRIPLE=x86_64-unknown-linux-gnu ;;
        Linux/aarch64) TRIPLE=aarch64-unknown-linux-gnu ;;
        *) echo "unsupported $OS/$ARCH"; exit 1 ;;
    esac
    echo "platform $OS/$ARCH -> $TRIPLE"
    curl -sSf -o "$HOME/rustup-init" "https://static.rust-lang.org/rustup/dist/$TRIPLE/rustup-init" || exit 1
    chmod +x "$HOME/rustup-init"
    "$HOME/rustup-init" -y --profile minimal --default-toolchain stable --target "$TRIPLE" || exit 1
    rm -f "$HOME/rustup-init"
    echo RUSTUP_DONE
} >"$LOG" 2>&1 </dev/null

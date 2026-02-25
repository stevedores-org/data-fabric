#!/usr/bin/env bash
set -euo pipefail

TOOLS_DIR="${PWD}/.worker-tools"

if [[ -d "$HOME/.cargo/bin" ]]; then
	export PATH="$HOME/.cargo/bin:$PATH"
fi

if ! rustc --print target-libdir --target wasm32-unknown-unknown >/dev/null 2>&1; then
	if ! command -v rustup >/dev/null 2>&1; then
		curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
		export PATH="$HOME/.cargo/bin:$PATH"
	fi

	if ! rustup target list --installed | grep -q '^wasm32-unknown-unknown$'; then
		rustup target add wasm32-unknown-unknown
	fi
fi

cargo install worker-build --locked --force --root "$TOOLS_DIR"
"$TOOLS_DIR/bin/worker-build" --release

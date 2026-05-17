#!/usr/bin/env bash
set -euo pipefail

if [[ -d "${HOME}/.cargo/bin" ]]; then
	export PATH="${HOME}/.cargo/bin:${PATH}"
fi

if ! command -v cargo >/dev/null 2>&1; then
	curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal
	export PATH="${HOME}/.cargo/bin:${PATH}"
fi

exec cargo run --package xtask -- build-worker

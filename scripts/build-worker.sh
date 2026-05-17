#!/usr/bin/env bash
set -euo pipefail

exec cargo run --package xtask -- build-worker

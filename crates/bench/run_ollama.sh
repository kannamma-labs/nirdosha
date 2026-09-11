#!/bin/sh
set -e
export NIRDOSHA_LLM_PROVIDER_KEY="ollama-local"
export NIRDOSHA_LLM_PROVIDER_MODEL="kimi-k2.7-code:cloud"
export NIRDOSHA_LLM_PROVIDER_BASE="http://localhost:11434/v1"
cd "$(dirname "$0")/../.."
cargo run -q -p nirdosha-bench

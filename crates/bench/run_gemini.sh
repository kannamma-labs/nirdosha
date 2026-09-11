#!/bin/sh
set -e
export NIRDOSHA_LLM_PROVIDER_KEY="$GOOGLE_API_KEY"
export NIRDOSHA_LLM_PROVIDER_MODEL="gemini-2.5-flash"
export NIRDOSHA_LLM_PROVIDER_BASE="https://generativelanguage.googleapis.com/v1beta/openai"
cd "$(dirname "$0")/../.."
cargo run -q -p nirdosha-bench

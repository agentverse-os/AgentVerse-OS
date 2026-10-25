#!/usr/bin/env bash
# on_grant llm (an external provider): the project gets OpenAI-, Anthropic- and Gemini-compatible endpoints through the project gate, the key — from
# the LLM_EXTERNAL_KEY setting (the gate inserts it itself too, so without a key we return a placeholder: clients require a non-empty one). stdout →
# the project env.
set -euo pipefail
key="${LLM_EXTERNAL_KEY:-}"
[[ -n "$key" ]] || key="none"
echo "OPENAI_BASE_URL=${GATE_URL}/v1"
echo "OPENAI_API_KEY=${key}"
echo "ANTHROPIC_BASE_URL=${GATE_URL}"
echo "ANTHROPIC_AUTH_TOKEN=${key}"
echo "GEMINI_BASE_URL=${GATE_URL}"
echo "GEMINI_API_KEY=${key}"
echo "LLM_API_KEY=${key}"

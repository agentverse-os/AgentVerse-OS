#!/usr/bin/env bash
# on_grant llm (внешний провайдер): проект получает OpenAI-, Anthropic- и Gemini-совместимые точки через gate проекта,
# ключ — из настройки LLM_EXTERNAL_KEY (gate подставляет его и сам, поэтому без ключа отдаём заглушку: клиенты требуют непустой). stdout → env проекта.
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

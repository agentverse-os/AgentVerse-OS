# Hermes Agent

The official [Hermes Agent](https://github.com/NousResearch/hermes-agent) image by Nous Research, latest version (v2026.8.31), with the
official Web Dashboard. Unlike the Coolify catalog variant (“Hermes Agent With Webui”: a third-party web chat on top of a three-month-old
agent), this is a single container and everything is native.

**What is inside**

- The dashboard behind “Open”: **Chat** — the agent terminal in the browser (xterm.js), **Sessions** — history with tool calls, **Config** —
  a `config.yaml` editor, **API Keys** — provider keys, **Skills/MCP**, **Cron**, **Logs**, **Analytics**, **Channels** — Telegram,
  Discord, Slack, WhatsApp, Signal, **System** — updates and maintenance.
- The messenger gateway runs all the time.
- The agent’s own OpenAI-compatible API at `http://hermes:8642` inside AgentVerse OS (the key is the “Hermes API key” setting).

**First run**

1. Open the app and sign in: user `admin`, the password from the login details.
2. LLM provider: either connect an LLM gateway (LiteLLM, Ollama) on the “Links” tab of the card (its address and key are put into the
   environment), or enter your own key on the API Keys tab of the dashboard. Then pick a model in Config.
3. Messengers — the Channels tab.

**Data** — `tank/apps/hermes-agent/data`: `.env`, `config.yaml`, `sessions/`, `memories/`, `skills/`. To update, change the image tag in
compose; the image migrates its config by itself.

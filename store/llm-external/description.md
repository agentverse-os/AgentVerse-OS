**External LLM endpoint** connects an OpenAI-compatible API that runs outside this server — LiteLLM, Ollama on a workstation, OpenAI itself — as the `llm` capability for projects. Nothing is deployed here: the core stores the address and the key, and each project's gate forwards `http://llm.gate` to that address, adding the key to the request headers.

**Setup**

1. Enter the address as `https://host[:port]` without a path; clients add `/v1` and the Anthropic or Gemini paths themselves. The server must reach the address (a tailnet name or a public host).
2. Enter the client API key if the provider needs one. For Ollama without a key leave it empty.
3. Install. The core checks that the address answers, then grants `llm` to every project that asks for it. In a project card you can pick which provider serves `llm` when several are installed.

Workspaces get `OPENAI_BASE_URL`, `ANTHROPIC_BASE_URL`, `GEMINI_BASE_URL` pointing at the gate and the key in `OPENAI_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `GEMINI_API_KEY`, `LLM_API_KEY`, the same contract as the local provider. Changing the address or key on the Settings tab updates the grants and gates at once.

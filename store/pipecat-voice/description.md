**Pipecat Voice** is a voice assistant that opens right in an AgentVerse OS window: press “Connect”, allow the microphone and talk.
Inside is [Pipecat](https://github.com/pipecat-ai/pipecat), the open framework for voice and multimodal agents by Daily, with its
built-in WebRTC client.

What the stack is made of:

- **STT** — faster-whisper via [speaches](https://github.com/speaches-ai/speaches), `Systran/faster-whisper-small` by default (fast on a CPU, understands Russian);
- **TTS** — Piper with Russian voices (irina, denis, dmitri, ruslan) in the same speaches; for English you can switch to Kokoro;
- **LLM** — any OpenAI-compatible address, for example **LiteLLM** or **Ollama** from this same list (the app address + `/v1` and a key in the settings), OpenAI or OpenRouter;
- **VAD** — Silero, pause detection right in the bot.

Everything runs locally except the LLM. The first install takes a few minutes: the Pipecat image is built and the models are downloaded.
Voice, language, models and the system prompt can be changed on the “Settings” tab and applied by redeploying.

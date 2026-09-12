Ollama allows you to download and run advanced AI models directly on your own hardware. Self-hosting AI models ensures full control over your data and protects your privacy.

⚠️ Before running a model, make sure your device has enough free RAM to support it. Attempting to run a model that exceeds your available memory could cause your device to crash or become unresponsive. Always check the model requirements before downloading or starting it.

**Getting Started:**
The easiest way to get started with Ollama is to install the Open WebUI app from the Umbrel App Store. Open WebUI will automatically connect to your Ollama setup, allowing you to manage model downloads and chat with your AI models effortlessly.

**Advanced Setup:**
If you want to connect Ollama to other apps or devices, here's how:
  - Apps running on UmbrelOS: Use ollama_ollama_1 as the host and 11434 as the port when configuring other apps to connect to Ollama. For example, the API Base URL would be: http://ollama_ollama_1:11434.
  - Custom Integrations: Connect Ollama to third-party apps or your own code using your UmbrelOS local domain (e.g., http://umbrel.local:11434) or your device's IP address, which you can find in the UmbrelOS Settings page (e.g., http://192.168.4.74:11434).
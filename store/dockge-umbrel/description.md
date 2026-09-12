⚠️ Make sure to only use named Docker volumes in your Compose files. Data in bind-mounted volumes will be lost when Dockge is restarted or updated.

⚠️ Watch out for port conflicts between your custom Docker containers and your umbrelOS apps.

Dockge is a fancy, easy-to-use and reactive self-hosted docker compose.yaml stack-oriented manager to run custom Docker  containers. It has an interactive editor for compose files and can convert docker run commands into docker-compose.yaml.

🛠️ Dockge on Umbrel is for power users, follow these best practices to avoid issues:

1. Data persistence: Make sure to only used named Docker volumes in your Compose files. Data in bind-mounted volumes will be lost when Dockge is restarted or updated.

2. Port management: Watch out for potential port conflicts between your custom containers and umbrelOS' service containers, apps you have installed from the Umbrel App Store or Community App Stores, and any apps you go to install in the future.

3. Container restart policy: Set your containers to "unless-stopped" or "always" restart policies. This will allow your containers to restart automatically when Dockge is restarted or updated.

4. Web access to containers: Access your custom containers in your browser at umbrel.local:PORT_NUMBER. For example, for a container with a web UI running on port 4545, navigate to umbrel.local:4545 to access it.
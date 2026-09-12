Databag is a self-hosted messaging service. Notable features include:

- Decentralized (direct communication between app and server node)
- Federated (accounts on different nodes can communicate)
- Public-Private key based identity (not bound to any blockchain or hosting domain)
- End-to-End encryption (the hosting admin cannot view sealed topics, deafult unsealed)
- Audio and Video Calls (nat traversal requires separate relay server)
- Topic based threads (messages organized by topic not contacts)
- Lightweight (server can run on a raspberry pi zero v1.3)
- Low latency (use of websockets for push events to avoid polling)
- Unlimited accounts per node (host for your whole family)
- Mobile alerts for new contacts, messages, and calls (supports UnifiedPush, FCM, APN)

🛠️ SET-UP INSTRUCTIONS 🛠️

🆕 Creating Accounts: You can create accounts and account links from the Admin Dashboard.

- To access the admin dashboard, click on the settings cog icon located at the upper right corner of the app login page.

- Use the "default app password" that is displayed on Databag's page in the app store (shown after install).

🌐 HTTPS Configuration: The browser version of the app can operate over http, but the mobile app requires https. You can set up https using the Cloudflare Tunnel app available in the app store. Detailed instructions for configuring the tunnel can be found here: https://github.com/Radiokot/umbrel-cloudflared/wiki/How-to-set-up-Cloudflare-Tunnel-on-your-Umbrel
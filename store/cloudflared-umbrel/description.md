Start a secure tunnel to access your Umbrel apps from the Internet using the Cloudflare network.  With Cloudflare Tunnel, you do not send traffic to an external IP — instead,  this app contains a lightweight tunneling daemon (cloudflared) that creates outbound-only  connections to Cloudflare's global network.

To use this app, you must have a Cloudflare account set up with your own domain(s) configured.  Once you have set up a Cloudflare account, check out this guide with examples to configure your own tunnel: https://github.com/Radiokot/umbrel-cloudflared/wiki/How-to-set-up-Cloudflare-Tunnel-on-your-Umbrel

⚠️ Apps accessible from the Internet incentivize attackers and bots to hack into them.  Only expose harmless apps or those having strong internal access control.

Powered by Cloudflare.
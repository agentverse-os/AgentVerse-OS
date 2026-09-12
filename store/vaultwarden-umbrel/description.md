Vaultwarden is an unofficial Bitwarden-compatible server for self-hosting your encrypted password vault on Umbrel. It works with many official Bitwarden clients and provides a lightweight alternative to the official Bitwarden server.

🛠️ **SET-UP REQUIRED**
Vaultwarden needs to be opened through HTTPS or Tor Browser for account creation. Many Bitwarden clients also require an HTTPS server URL. The easiest private HTTPS option is Tailscale Serve, which gives Vaultwarden an HTTPS URL that only devices in your Tailscale network can access.

**Option 1: Set up with Tailscale Serve**
- Install and sign in to the Tailscale app on your Umbrel.
- In the Tailscale admin console, enable MagicDNS and HTTPS Certificates.
- On your Umbrel, go to Settings > Advanced settings > Terminal and select the Tailscale app.
- Run `tailscale serve --bg --https=443 http://127.0.0.1:8089`.
- Open the HTTPS URL shown by Tailscale and create your Vaultwarden account.
- Use that same HTTPS URL as the self-hosted server URL in Bitwarden clients.
> **Note:** Clicking the Vaultwarden app icon in the Umbrel dashboard may still show a Tor access warning. When using Tailscale Serve, open the Tailscale HTTPS URL directly instead.

**Option 2: Set up with Tor**
- Ensure your Umbrel is accessible over Tor by going to Settings > Advanced settings and toggling "Remote Tor access" to enabled. You will need to restart your Umbrel if Remote Tor access was previously disabled.
- Access your Umbrel through Tor Browser using the .onion address shown in Settings under "Remote Tor access".
- In Tor Browser, open the Vaultwarden app and create an account.

🔐 **CONNECTING BITWARDEN CLIENTS**
Official Bitwarden clients may reject plain HTTP URLs, including http://umbrel.local, plain Tailscale IP addresses, and onion HTTP URLs. Use the Tailscale Serve HTTPS URL or another HTTPS reverse proxy URL for client connections.

**Features:** Vaultwarden supports personal vaults, organizations, attachments, Bitwarden Send, website icons, two-factor authentication, emergency access, and the Vaultwarden admin backend.

**Disclaimer:** Vaultwarden is not associated with Bitwarden or 8bit Solutions LLC. For Vaultwarden issues, contact Vaultwarden/Umbrel support rather than Bitwarden's official support channels.
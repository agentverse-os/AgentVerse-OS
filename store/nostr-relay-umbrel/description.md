Introducing Nostr Relay — an official app by Umbrel.

Step 1. Connect your Nostr client (e.g., Damus, Amethyst) to your private relay for seamless backup of all Nostr activity. In Damus, add your Relay URL via Menu > Relays.

Tip: Install Tailscale on your Umbrel and your devices for an uninterrupted connection between your clients and your relay, even when you're away from your home network. Enable Tailscale's MagicDNS and use ws://umbrel:4848 as your Relay URL.

Step 2. Tap the globe icon on the top to back up past Nostr activity from your public relays and ensure uninterrupted future backups, even if the connection between your private relay and Nostr client is disrupted.

That's it! Your past and future Nostr activity will be now backed up to your private relay.

Nostr Relay is powered by the open source nostr-rs-relay project — a Rust implementation of Nostr relay. It currently supports the entire relay protocol, including NIP-01, NIP-02, NIP-03, NIP-05, NIP-09, NIP-11, NIP-12, NIP-15, NIP-16, NIP-20, NIP-22, NIP-26, NIP-28, and NIP-33.
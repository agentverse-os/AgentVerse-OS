bitmagnet is a self-hosted BitTorrent indexer with a web UI, GraphQL API, and Servarr integration.

It crawls the BitTorrent DHT and indexes torrent metadata so you can search your own private index.

🧩 TORZNAB / SERVARR INTEGRATION

bitmagnet exposes a Torznab-compatible API at `/torznab` that you can add to Prowlarr and other Torznab-compatible apps on Umbrel.

In Prowlarr: Add Indexer → Generic Torznab

Torznab URL example:
  - http://umbrel.local:3335/torznab


If you are configuring from another container on the same Docker network, you can typically use:
  - http://bitmagnet_bitmagnet_1:3333/torznab


⚠️ bitmagnet currently has no authentication/access control; do not expose it publicly.
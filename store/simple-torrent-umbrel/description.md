SimpleTorrent is a a self-hosted remote torrent client that starts torrents remotely, download sets of files on your Umbrel, which are then retrievable or streamable via web browser over HTTP. This project is a re-branded fork of cloud-torrent by jpillora. Features:

- Individual file download control
- Run external program on tasks completion: DoneCmd
- Stops task when seeding ratio reached: SeedRatio
- Download/Upload speed limiter: UploadRate/DownloadRate
- Detailed transfer stats in web UI.
- Torrent Watcher
- Extra trackers from external source
- Protocol Handler to magnet:
- Magnet RSS subscribing supported

⚠️ SimpleTorrent downloads torrents over the Clearnet, not Tor.
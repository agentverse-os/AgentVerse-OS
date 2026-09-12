qBittorrent is an open-source software alternative to µTorrent. It's designed to meet the needs of most users while using as little CPU and memory as possible.

🛠️ SET-UP INSTRUCTIONS

qBittorrent on umbrelOS is set up to work without any additional configuration needed. It will automatically be connected to dependent apps like Radarr, Sonarr, Lidarr, Readarr, and Prowlarr. Simply install additional media apps from the Umbrel App Store, and everything will work seamlessly together.

Some additional tips:

- Please make sure that you do not change the default download path in the app settings. It should remain set to "/downloads" to ensure that your downloads show up in your main Umbrel downloads folder.
- It is recommended to change the default password for the app after installation.
- This app comes bundled with two alternative Web UI's: VueTorrent and Nightwalker. To enable them, navigate to tools --> options --> Web UI and select "Use alternative Web UI". In the "Files location" field, enter "/app/vuetorrent" for VueTorrent or "/app/nightwalker" for Nightwalker and then click "Save". 

⚠️ qBittorrent downloads torrents over the Clearnet, not Tor.
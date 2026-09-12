OpenThread Border Router (OTBR) lets you bridge a Thread mesh network to your local Wi-Fi/Ethernet network using a Thread radio dongle. It enables Home Assistant and Matter-over-Thread devices (locks, sensors, plugs, thermostats) to communicate with your smart home stack without relying on a vendor-specific hub like Apple HomePod, Google Nest Hub, or Amazon eero.

## Setup
1. Plug your Thread radio dongle into your Umbrel device.
2. Open the app. The setup wizard detects
   your radio's serial device, lets you confirm the backbone network
   interface, and checks that IPv6 is available on your host and LAN. Pick
   the device and save — the border router then starts automatically.

3. Open Home Assistant and add the "OpenThread Border Router"
   integration, using the REST API URL shown in this app after setup. The
   OTBR web dashboard link is also shown in the app after setup.


## Notes
- **IPv6 is required.** Thread relies on end-to-end IPv6, so IPv6
  must be enabled on your Umbrel host and your router/LAN. The setup wizard
  verifies this.
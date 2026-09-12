EE Gateway turns your Umbrel into a self-hosted Bluetooth gateway for open BLE networks.

It scans with your device's Bluetooth radio for nearby BLE devices from supported networks and forwards their packets to the right upstream cloud, extending ground coverage from hardware you already own. Hubble Network is supported today; more partner networks will be added as they come online.

Two containers do the work: a worker that scans over Bluetooth and ingests packets upstream, and a small web dashboard for first-time setup and live status. You'll need a free encryptedenergy.com account to create the API token used during setup. Enter it once in the setup wizard, and the gateway runs on its own.

EE Gateway is free and open-source software (GPL-3.0-only), built by encryptedenergy.com. It is an independent community project and is not affiliated with any specific BLE network.
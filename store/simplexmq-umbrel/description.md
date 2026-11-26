SimpleX Chat is a private messenger with no user IDs or phone numbers. Your messages and files travel to your contacts through relay servers. SimpleX Server runs your own relays on your Umbrel, so they pass through hardware you control instead of someone else's servers:

- an SMP server, which relays your messages
- an XFTP server, which relays files, images and voice messages

Both servers get their own Tor address automatically, so you don't need a domain name or to open ports on your router. Contacts who don't use Tor can usually still reach you, because their app forwards messages to your server using SimpleX's private message routing. The server addresses include a password, so only you can create new conversations or upload files on your servers. Messages that haven't been delivered are deleted after 21 days, and files after 48 hours.

To add your servers to SimpleX Chat:

1. Right-click SimpleX Server on your Umbrel, open Settings → Advanced → View logs, and copy the two addresses that start with smp:// and xftp://.
2. Your phone or computer needs Tor to reach the .onion addresses. On a phone, install Orbot. On a computer, run Tor.
3. In SimpleX Chat, open Settings → Network & servers → Advanced network settings and turn on Use SOCKS proxy. Then add both addresses as your own servers in Network & servers and test them.

If you have a public domain or IP address, add it in SimpleX Server's Settings → Advanced → Environment variables and forward ports 5223 and 5443 so devices without Tor can connect directly. The page that opens from your Umbrel shows your SMP server's status and settings.
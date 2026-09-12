Expose your apps to the internet easily and securely.

⚠️ Be cautious when exposing apps to the public internet. Ensure they have proper security, such as login protection, and avoid exposing sensitive apps without adequate safeguards.

🔧 Nginx Proxy Manager uses port 40080 for HTTP (unsecured) traffic and port 40443 for HTTPS (secured) traffic. To make your apps accessible from the public internet, you will need to set up port forwarding on your router. Forward external port 80 (HTTP) to internal port 40080 and external port 443 (HTTPS) to internal port 40443.

🔍 Features:

  - Beautiful and Secure Admin Interface based on Tabler
  - Easily create forwarding domains, redirections, streams and 404 hosts without knowing anything about Nginx
  - Free SSL using Let's Encrypt or provide your own custom SSL certificates
  - Access Lists and basic HTTP Authentication for your hosts
  - Advanced Nginx configuration available for super users
  - User management, permissions and audit log
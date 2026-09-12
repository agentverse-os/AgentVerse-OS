FlareSolverr is a proxy server designed to bypass anti-bot and DDoS protection from Cloudflare and DDoS-GUARD. It operates by starting a proxy server and waiting for user requests.

When a request is received, it uses Selenium with undetected-chromedriver to launch a headless browser (Chrome). This browser navigates to the requested URL, automatically solving the security challenges.  Once the challenge is cleared, the HTML code and the necessary cookies are sent back to the user. 

These returned cookies can then be used with other HTTP clients to access the protected content directly. It supports both temporary and permanent user sessions.

Note: Use http://flaresolverr_server_1:8191/v1 as the URL for integration with other applications.
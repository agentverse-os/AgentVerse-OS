Gitea Mirror automatically mirrors your GitHub repositories to a self-hosted Gitea instance. It supports mirroring public and private repositories, organizations, starred repos, issues, wikis, and pull requests.

Configure your GitHub and Gitea credentials through the web interface to get started. The app will periodically sync your repositories based on your configured schedule.

Key features include:
  - Mirror repositories from GitHub to Gitea
  - Support for organizations and starred repositories
  - Issue and wiki mirroring
  - Configurable sync intervals and scheduling
  - Repository cleanup and orphan management
  - Header-based authentication for reverse proxy SSO


**Connecting to Gitea on Umbrel:** If you have the Gitea app installed on your Umbrel, use `http://gitea_server_1:8085` as the Gitea URL in the mirror settings.

The **Gitea Access Token** should have the following permissions:
  - write:repository
  - read:user
  - write:organization (optional)
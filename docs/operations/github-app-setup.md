# GitHub App setup for local development

Use a dedicated test organization and repository. No Docker is required for the tunnel.

1. Start the API on port 23000.
2. Run `pnpm dev:tunnel` (wraps `ngrok http 23000`; set `NGROK_DOMAIN` for a reserved domain). Configure ngrok once with `ngrok config add-authtoken <token>`; the script never reads the token.
3. Register the App from `infra/github/app-manifest.json` (manifest flow). Permissions: contents read, pull_requests write, checks write, metadata read. No administration and no contents write; the App cannot merge.
4. Set the webhook URL to `https://<subdomain>.ngrok-free.app/api/v1/webhooks/github`.
5. Generate a secret locally: `node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"` and store it as `GITHUB_WEBHOOK_SECRET` in `.env` (gitignored).
6. Download the private key to `.secrets/github-app.pem` (gitignored) and set `GITHUB_APP_PRIVATE_KEY_PATH`.
7. Install the App on the test repository and open a pull request. A 202 appears in the ngrok inspector at http://127.0.0.1:4040.

## Safety

- Rotate the secret if exposed; never paste keys into issues or chats.
- Free-tier ngrok URLs change per run; update the App webhook URL each time.
- The ngrok inspector shows payloads, so use only synthetic or test-repository data.
- Signature verification stays mandatory over the tunnel. Stop the tunnel when not in use.

Offline alternative: `node tools/dev/replay-webhook/index.mjs --file payload.json` sends signed webhooks without a real App.

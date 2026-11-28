# Publishing to the MCP Registry

`server.json` in the repo root lists the hosted hotpath-cloud MCP server (`https://hotpath.rs/api/v1/mcp`) in the [official MCP Registry](https://registry.modelcontextprotocol.io) as `rs.hotpath/hotpath-cloud`. It is a remote-only entry, so the file has no `packages`. The server implementation lives in the private backend, so `repository` names this public repo (the client, docs and the entry itself), with GitHub's stable repo `id` (`gh api repos/hotpath-rs/hotpath --jq .id`). Its `version` is independent of the crate versions. Publishing is manual, there is no CI workflow for it.

1. Install the CLI: `brew install mcp-publisher` (or a release binary from [modelcontextprotocol/registry](https://github.com/modelcontextprotocol/registry/releases)).
2. Validate: `mcp-publisher validate server.json`.
3. Generate a signing key **outside the repository** and keep it in a password manager. macOS LibreSSL has no Ed25519, so use OpenSSL 3 (`brew install openssl@3`):

   ```bash
   mkdir -p ~/secure
   chmod 700 ~/secure
   OPENSSL="$(brew --prefix openssl@3)/bin/openssl"
   KEY=~/secure/hotpath-mcp-registry.pem
   (umask 077 && "$OPENSSL" genpkey -algorithm Ed25519 -out "$KEY")
   chmod 600 "$KEY"
   PUBLIC_KEY="$("$OPENSSL" pkey -in "$KEY" -pubout -outform DER | tail -c 32 | base64)"
   echo "hotpath.rs. IN TXT \"v=MCPv1; k=ed25519; p=${PUBLIC_KEY}\""
   ```

4. Add that TXT record to the `hotpath.rs` apex (name `@` or `hotpath.rs`, not a subdomain), next to the existing TXT records, and wait until `dig +short TXT hotpath.rs` shows it.
5. Log in with DNS auth, which grants the `rs.hotpath/*` namespace. The variables are set again so this step also works in a new shell:

   ```bash
   OPENSSL="$(brew --prefix openssl@3)/bin/openssl"
   KEY=~/secure/hotpath-mcp-registry.pem
   PRIVATE_KEY="$("$OPENSSL" pkey -in "$KEY" -noout -text | grep -A3 "priv:" | tail -n +2 | tr -d ' :\n')"
   mcp-publisher login dns --domain hotpath.rs --private-key "${PRIVATE_KEY}"
   ```

   The CLI stores its Registry token in your home directory, never in the repo.
6. Publish from the repo root: `mcp-publisher publish`.
7. Verify the listing: `curl "https://registry.modelcontextprotocol.io/v0.1/servers?search=rs.hotpath/hotpath-cloud"`.

To update the listing, change `server.json`, bump its `version` (published versions are immutable), validate, log in again if the token expired, and publish. Keep the TXT record in place, every login checks it.

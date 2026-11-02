# Secrets management

Secrets is a standalone workspace beside Clusters. It is not a Catalog category or
Preferences section. The **+** action opens **Add secret connection**. Vault KV v2
is the first supported provider; the Rust provider interface and credential-store
interface are separate so additional providers do not need Vault's transport.

## First connection

1. Open **Secrets**, then **+**.
2. Enter a connection name and HTTPS origin, for example
   `https://vault.example.com:8200`. URL credentials, query strings, fragments,
   reverse-proxy path prefixes, and HTTP endpoints are not supported.
3. Set the optional Vault namespace, userpass auth mount (default `userpass`), KV v2
   mount (default `secret`), and optional base path. Mount and base paths are
   relative paths without leading/trailing slashes or traversal components.
4. Save the connection. Saving metadata does not connect or authenticate.
5. Open the connection and log in with a Vault userpass username and password.
6. Optionally enable **Remember login** before logging in. Only the issued token
   is saved in the OS credential store. The password is not saved.
7. After login, choose a discovered KV v2 engine, then browse folders and paths.
   Select a secret to open its metadata and version inspector on the right. Choose
   **Reveal** explicitly to display that version's fields.
8. Use **Edit**, **Pin**, or **Remove** on a saved connection. Multiple connections
   can be open at once; switching connections preserves independent Rust sessions,
   while hidden browser passwords and revealed values are cleared.

The Vault identity needs LIST access on the configured KV v2 `metadata` paths and
read access on the corresponding `data` paths. Saved-login resume also needs
`auth/token/lookup-self`. The app does not create policies or change permissions.
A list denial, missing path, unavailable server, or malformed response is an error,
not an empty successful inventory. A permitted read of a known path does not
imply permission to list that path's parents.

## Credentials and privacy

Rust owns provider HTTPS requests, profile persistence, authentication, and tokens.
The webview temporarily handles the submitted password and values explicitly
requested through Reveal. It never receives the Vault token, arbitrary Vault
responses, or upstream error bodies. IPC is typed and bound to the profile,
native window, session, and monotonic read ID. Browser preview and the development
engine bridge cannot invoke Secrets operations.

- Profile metadata lives in `secrets/secrets-profiles.sqlite3` below the app-data
  directory. It contains the provider, name, endpoint, namespace, mounts, and base
  path, not credentials. Pin state is metadata only and uses the same Rust-owned
  profile store. These metadata fields can still identify internal infrastructure;
  this is not an encrypted profile database.
- **Remember login** uses macOS Keychain, Windows Credential Manager, or Linux's
  desktop Secret Service. Linux requires a working, unlocked Secret Service
  session. There is no plaintext-file, SQLite, or in-memory-only fallback for a
  requested persistent credential write. Locked/denied/unavailable store access
  is reported; login can be retried without Remember login.
- Saved tokens are addressed by an opaque key bound to the complete profile and
  local app-data path. Copying a profile database to another data directory does
  not automatically reuse credentials from the original installation.
- **Resume saved login** is explicit and validates the stored token with Vault
  before enabling reads. Tokens are not renewed automatically.
- Disconnect clears the in-memory session credential and cancels reads, but keeps
  any remembered login. **Forget saved login** also deletes the bound stored
  credential. Removing a profile attempts credential deletion before deleting its
  metadata; if the OS store cannot complete deletion, the profile is retained so
  forgetting can be retried.
- Closing the connection/page or native window clears its session. Expiry, 30
  minutes of inactivity, and app shutdown clear credentials and cancel work.
  Disconnect/forget are local cleanup, not a Vault token-revocation operation.
- Editing preserves the profile ID and pin but disconnects that profile and removes
  its remembered token. If forgetting fails, the edit is not saved. Pin changes do
  not read/write credentials or disturb sessions.
- Revealed values are hidden after at most 30 seconds, when navigating/hiding,
  closing the view, or on window blur. JavaScript/IPC/HTTP serialization can create
  copies: hiding and zeroizing owned Rust buffers are not a guarantee of erasing
  every allocation, preventing screenshots, or protecting a compromised OS.

## Supported scope and limits

This first version supports username/password login, saved token resume, authenticated
KV v2 engine discovery, folder listing, metadata/version history, and explicit latest-
version reads. The remote operations are userpass login, token lookup, mount metadata,
KV metadata LIST, KV metadata GET, and KV data GET. It does not write, delete, rotate,
inject, copy, export, or automatically reveal secrets. It does not modify Kubernetes
authentication.

Only verified HTTPS is accepted. Redirects and environment-derived proxies are
disabled; no skip-TLS-verification option is provided. Custom CA files, custom
proxies, OIDC, AppRole, MFA challenges, root-policy tokens, non-expiring tokens,
KV v1 and automatic token renewal are not implemented. Unsupported discovered
engines are shown but cannot be browsed. Editing a connection preserves its identity
and pin, invalidates its sessions, and forgets its saved login before saving changes.

Responses have a 1 MiB bound and a 15-second deadline (5-second connect timeout).
Lists show at most 1,000 immediate entries and expose a partial flag if more were
returned. Reveals allow at most 256 fields. Nested JSON field values are displayed
as plain text, never interpreted as HTML. There are at most 100 saved profiles
and one Secrets session per native window/profile pair, up to 32 concurrent sessions.
Version history shows at most 200 records with an explicit partial indicator.

## Verification and remaining native checks

Rust coverage in `src-tauri/src/secrets/` uses synthetic provider/credential-store
implementations, temporary profile databases, and loopback HTTP fixtures that are
allowed only in tests. It covers profile bounds and symlink rejection, redacted
errors, request/response limits, cancellation, cross-window isolation, expiry,
remember/resume/forget failures, and delayed credential-write cleanup.

`tests/secrets.spec.ts`, `tests/secrets-api.spec.ts`, and
`tests/route-restoration.spec.ts` use mocked IPC for dialog/navigation, browser
fallback, scope validation, explicit reveal, keyboard recovery, pending operation
disposal, and expiry. They do not verify native secure-store behavior or a real
Vault deployment. Last-page restoration remembers only `/secrets`, not a selected
profile, session, username, secret path, or revealed value.

Before claiming native platform verification, manually test a disposable scoped
Vault account and test credential entries on each target OS: login without
remembering, denied/locked store access, remember/restart/resume, expiry,
forget/remove, two native windows, cancellation, and app shutdown. Do not run
these against real user credentials or clusters as automated agent tests.

## References

- [Vault userpass HTTP API](https://developer.hashicorp.com/vault/api-docs/auth/userpass)
- [Vault KV v2 HTTP API](https://developer.hashicorp.com/vault/api-docs/secret/kv/kv-v2)
- [Vault token HTTP API](https://developer.hashicorp.com/vault/api-docs/auth/token)
- [keyring 3.6.3](https://docs.rs/keyring/3.6.3/keyring/)

This workflow is based on these provider APIs, not adapted Freelens code. No
Freelens extension compatibility or support for unimplemented providers is claimed.

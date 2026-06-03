# API reference — `als-api` typed client

Endpoint-by-endpoint reference for the typed Rust wrappers in
`crates/als-api/src/endpoints/`. The wire-level contract (request
bodies, response envelopes, error codes) is owned by
`cli-server-contract.md` in the als-docs repo; this file documents
how the CLI binds to that contract.

All success responses are wrapped by the server in
`{ success: true, data: ... }`; `Client::get_json` /
`Client::post_*` strip the envelope so the typed return type matches
the inner `data` payload. The two device-flow endpoints
(`authorize` / `poll`) bypass the envelope because RFC 8628 defines
its own plain JSON shapes — `Client::unauth_post_form_raw` parses
those directly.

All errors are surfaced as `ApiError`; `From<ApiError> for
als_core::Error` maps them onto exit codes per
[`cli-spec.md`](cli-spec.md).

## Authentication

Every endpoint except the two device-flow ones requires
`Authorization: Bearer <token>`. The token is loaded from the active
profile in `~/.config/als/config.toml` (or `$XDG_CONFIG_HOME/als/`)
by `als-core::config::load` and injected by `Client::new`.

## Endpoint map

| Module      | Method   | Path                              | Function (`endpoints::<mod>::<fn>`) |
|-------------|----------|-----------------------------------|--------------------------------------|
| `device`    | `POST`   | `/api/auth/device/authorize`      | `device::authorize`                  |
| `device`    | `POST`   | `/api/auth/device/token`          | `device::poll`                       |
| `account`   | `GET`    | `/api/account/me`                 | `account::me`                        |
| `quota`     | `GET`    | `/api/quota/me`                   | `quota::me`                          |
| `sites`     | `GET`    | `/api/sites`                      | `sites::list`                        |
| `sites`     | `GET`    | `/api/sites/:id`                  | `sites::get`                         |
| `sites`     | `PATCH`  | `/api/sites/:id`                  | `sites::patch`                       |
| `sites`     | `DELETE` | `/api/sites/:id`                  | `sites::delete`                      |
| `password`  | `POST`   | `/api/sites/:id/password`         | `password::set`                      |
| `versions`  | `GET`    | `/api/sites/:id/versions`         | `versions::list`                     |
| `versions`  | `POST`   | `/api/sites/:id/activate`         | `versions::activate`                 |
| `deploy`    | `POST`   | `/api/deploy` (multipart)         | `deploy::deploy`                     |
| `tokens`    | `DELETE` | `/api/tokens/:prefix`             | `tokens::delete`                     |

## Module: `device` — RFC 8628 device authorization

### `device::authorize`

Start a device-flow grant. Returns the user-code, verification URL,
and polling interval.

```rust
let auth: DeviceAuthorization =
    device::authorize(&client, hint.as_deref()).await?;
```

`hint` is the operator's hostname (`hostname::get()` on Unix /
Windows, env `HOSTNAME` first). Server-side it is recorded against
the issued token so revocation lists show a meaningful label.

Form-encoded request body: `client_id=toss-cli&hostname_hint=<host>`.

`DeviceAuthorization` fields:

| Field                       | Type     | Notes |
|-----------------------------|----------|-------|
| `device_code`               | `String` | Opaque; passed back to `poll`. |
| `user_code`                 | `String` | Short code typed into the browser. |
| `verification_uri`          | `String` | Plain URL the user types. |
| `verification_uri_complete` | `String` | One-tap URL with the code embedded. |
| `expires_in`                | `u64`    | Seconds until the device_code expires. |
| `interval`                  | `u64`    | Seconds between polls; server may bump. |

### `device::poll`

Poll once for the access token. The CLI loops on this, sleeping
`interval` seconds between calls, doubling on `slow_down` (capped at
30 s), and bailing on `access_denied` / `expired_token`.

```rust
match device::poll(&client, &device_code).await? {
    DeviceTokenResponse::Granted { access_token, .. } => { /* persist */ }
    DeviceTokenResponse::Pending => { /* continue polling */ }
    DeviceTokenResponse::SlowDown => { /* double interval */ }
    DeviceTokenResponse::Denied => { /* exit 1 */ }
    DeviceTokenResponse::Expired => { /* exit 1 */ }
}
```

Form-encoded request body:
`grant_type=urn:ietf:params:oauth:grant-type:device_code&device_code=<code>&client_id=toss-cli`.

## Module: `account`

### `account::me`

```rust
let me: AccountMe = account::me(&client).await?;
```

Returns the currently authenticated user record. The server returns
a richer payload (`groups`, TOTP state, etc.); the CLI only models the
fields it renders and leans on serde's "ignore unknown fields" default
for the rest.

| Field            | Type              | Notes |
|------------------|-------------------|-------|
| `id`             | `String`          | Stable user id. |
| `username`       | `String`          | Login name. |
| `name`           | `String`          | Display name. |
| `email`          | `String`          | |
| `role`           | `String`          | `user` / `admin` / etc. |
| `avatar`         | `Option<String>`  | Optional avatar URL. |
| `status`         | `Option<String>`  | `active` / `suspended` / etc. |
| `last_login_at`  | `Option<String>`  | ISO-8601. |
| `created_at`     | `Option<String>`  | ISO-8601. |

## Module: `quota`

### `quota::me`

```rust
let q: Quota = quota::me(&client).await?;
```

Per-user quota counters consumed by `als auth status`'s footer.

| Field                  | Type            | Notes |
|------------------------|-----------------|-------|
| `total_sites`          | `u32`           | Current count of active sites. |
| `max_sites`            | `u32`           | Hard cap on site count. |
| `total_bytes`          | `u64`           | Cumulative storage used (server-tracked). |
| `max_size_bytes`       | `u64`           | Per-archive (upload) cap. |
| `max_extracted_bytes`  | `Option<u64>`   | Optional extracted-size cap. |
| `max_files_per_site`   | `Option<u32>`   | Optional per-site file-count cap. |

## Module: `sites`

### `sites::list`

```rust
let resp: ListSitesResponse =
    sites::list(&client, &ListSitesQuery { limit, page, all }).await?;
```

Wire query parameters (the `state` / `auth` filtering shown by `als
list` is applied client-side after the response — see
`commands/list.rs`):

| Param   | Type          | Notes |
|---------|---------------|-------|
| `page`  | `Option<u32>` | 1-based page index. Omitted ⇒ server default. |
| `limit` | `Option<u32>` | Rows per page. Server cap is 200. |
| `all`   | `bool`        | Admin-only: include every active site, not just the caller's. Silently ignored for non-admins. |

`ListSitesResponse` is `{ data: Vec<Site>, meta: PaginationMeta }`.

### `sites::get` / `sites::delete`

```rust
let site: Site         = sites::get(&client, id).await?;
let ()                 = sites::delete(&client, id).await?;
```

`id` is the 10-char base32 site id (e.g. `k7x2qm4j6p`).

### `sites::patch`

Partial update. Any non-`None` field is forwarded; omitted fields are
left unchanged on the server. `Option<Option<T>>` on `description` /
`expires_at` distinguishes "leave alone" (`None`) from "clear"
(`Some(None)`). Returns the full updated row.

```rust
let updated: Site = sites::patch(&client, id, &PatchSiteRequest {
    project_name: Some("new-name".into()),
    description:  None,
    expires_at:   Some(Some("2026-12-31T00:00:00Z".into())),
}).await?;
```

Rolling forwards / backwards to a past version goes through
`versions::activate`, not `patch` — there is no `version` field on the
PATCH body.

## Module: `password`

### `password::set`

One POST endpoint covers set / replace / clear via the optional
`password` argument:

| Value                 | Effect |
|-----------------------|--------|
| `Some("auto")`        | Server generates a password. The server does not currently echo the plaintext back, so the binary mints memorable passwords client-side and sends them as literals when it needs to display one. |
| `Some("<plaintext>")` | Server stores it verbatim. |
| `None`                | Clears the password (site becomes public). |

```rust
let resp: SetPasswordResponse =
    password::set(&client, id, password).await?;
```

`SetPasswordResponse` carries the updated `Site` row (flattened) plus
an optional `op` audit discriminator (`"set"` / `"cleared"` /
`"rotated"`).

## Module: `versions`

### `versions::list` / `versions::activate`

```rust
let rows: Vec<Version> = versions::list(&client, id).await?;
let ()                 = versions::activate(&client, id, version_id).await?;
```

Each `Version` carries the id, deployment timestamp, archive size,
and the operator who pushed it. `activate` is used by both rollback
and roll-forward; the server enforces that the version belongs to
the site.

## Module: `deploy`

### `deploy::deploy`

```rust
let resp: DeployResponse = deploy::deploy(
    &client,
    zipped_bytes,
    "site.zip".to_string(),
    DeployRequest {
        site_id:       Some("k7x2qm4j6p".into()),
        project_name:  Some("myproj".into()),
        expires:       Some(als_core::parse_duration("7d")?),
        auth_password: Some("auto".into()),
    },
).await?;
```

Multipart request body. The form fields are `archive` (the zip
bytes), `site_id`, `project_name`, `expires`, and `auth_password`.
All four metadata fields are optional.

Identity resolution, server-side:

- **`site_id` present** — authoritative anchor. If the id is owned
  by the caller, the upload pushes a new version to that site and
  the request's `project_name` is **not** used for identification.
  When `project_name` is also present and differs from the site's
  current name, the server treats it as a rename hint and applies
  it in the same request (subject to per-user uniqueness); when
  absent, the server keeps its current name. Either way the
  response surfaces the resulting name so the CLI can refresh a
  stale local pin. Unknown or non-owned id → `404 site_not_found`.
- **`site_id` absent** — case-insensitive per-user name dedup on
  `project_name`. A hit pushes a new version, a miss allocates a
  fresh site.

The CLI sources `site_id` from (highest priority first) the
committed `als.toml.id` in the input directory, then the local pin
store entry for the canonical input path; only `--new` skips both.
After a successful deploy the response's `id` is written back into
`als.toml` so the field is self-populating across machines.
`project_name` is sent on every unanchored deploy (user-supplied
via `--name`, read from `als.toml.name`, or locally minted as
`<word><NNN>`) and also on anchored deploys when the user
explicitly passes `--name` (rename intent). The no-name branch on
the server is therefore unreachable from this client.

Response fields:

- `resp.id` — site id (10-char base32; unchanged on a version push).
- `resp.url` — published URL (unchanged on a version push).
- `resp.project_name` — final name (echoes what the server stored).
- `resp.version` — the new version id (always fresh).
- `resp.expires_at` — ISO-8601 timestamp, or `null` for never.
- `resp.auth_required` — `true` when a visitor password is set.

## Module: `tokens`

### `tokens::delete`

```rust
let () = tokens::delete(&client, token_prefix).await?;
```

Used by `als auth revoke`. `prefix` is the first 8 chars of the
token (matches what the server displays in token listings).

## Error envelope

Every non-2xx response (and any 2xx that fails envelope parsing) is
turned into `ApiError`:

```rust
pub enum ApiError {
    Status   { status: u16, code: String, message: String },
    Network  { source: reqwest::Error },
    Encoding { source: serde_json::Error },
    Auth,
}
```

`From<ApiError> for als_core::Error` maps `code` strings onto the
exit-code matrix in [`cli-spec.md`](cli-spec.md). Notable codes:

| Code                       | Exit | Meaning |
|----------------------------|------|---------|
| `unauthorized`             | 1    | Missing / invalid bearer token. |
| `not_found`                | 1    | Site / version / token not found. |
| `archive_too_large`        | 1    | Deploy archive exceeds the per-archive cap. |
| `quota_exceeded`           | 1    | User over their site or storage quota. |
| `login_denied`             | 1    | Device flow returned `access_denied`. |
| `login_timeout`            | 1    | Device flow returned `expired_token` (or transport errors). |

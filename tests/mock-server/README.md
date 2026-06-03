# toss-mock-server

Bun + Hono mock of the toss server. Single source of HTTP truth for both
crate-level integration tests (`crates/als-api/tests/`) and CLI E2E tests
(`crates/als/tests/`). The runtime, framework, and envelope semantics match
the production server so mock behavior tracks real behavior.

See the `cli-server-contract.md` in the als-docs repo for the wire
contract this mock mirrors and
[`../../docs/development.md`](../../docs/development.md#mock-server-lifecycle)
for the Rust harness invocation pattern.

## Install

```bash
cd tests/mock-server
bun install
```

`bun.lock` is committed; CI uses `bun install --frozen-lockfile`.

## Run

```bash
bun run src/main.ts --port 0
```

`--port 0` asks the OS for an ephemeral port. Pass `--port 8123` to bind a
fixed port (rarely useful — the Rust harness always uses `0`).

## Handshake Protocol

On start, the process prints exactly one line to **stdout**:

```
LISTENING http://127.0.0.1:<actualPort>\n
```

All other output goes to **stderr** (`console.error` / `process.stderr.write`).
The Rust harness reads the first stdout line, strips `LISTENING `, parses the
URL, and uses it as the API base for every subsequent test request.

The server binds `127.0.0.1` only — never `0.0.0.0`. SIGTERM and SIGINT trigger
a clean shutdown with exit code `0`.

## Public Endpoints

| Method | Path        | Description                       |
|-------:|-------------|-----------------------------------|
| GET    | `/healthz`  | Liveness probe. Returns `{ok:true}`. |

Business routes mount under `/api/*` and mirror the production wire
contract documented in the als-docs repo (`cli-server-contract.md`).

## Control Endpoints (`/__test/*`)

These exist **only** in the mock and must never appear on the production
server. They are not advertised anywhere outside this README.

| Method | Path             | Body                                            | Behavior |
|-------:|------------------|-------------------------------------------------|----------|
| POST   | `/__test/reset`  | (none)                                          | Clears all maps and re-seeds fixtures. Returns `{ok:true,data:{reset:true}}`. |
| POST   | `/__test/seed`   | `{ users?, tokens?, sites?, connect_sessions? }` | Merges arrays into state, keyed by id / full / session_id. |
| POST   | `/__test/inject` | `{ path: "POST /v1/deploy", status: 413, code: "archive_too_large", message? }` | One-shot failure injection consumed by the first matching `/v1/*` handler. |
| GET    | `/__test/state`  | (none)                                          | JSON snapshot of state for debugging. |

`/__test/seed` and `/__test/inject` validate their request body and return
`invalid_request` (HTTP 400) on malformed input.

## Default Fixtures

`POST /__test/reset` re-seeds the following baseline:

- **User**
  - `id`: `u_alice`
  - `email`: `alice@example.com`
  - `name`: `Alice Liu`
  - `quota.max_sites`: `100`, `quota.max_total_bytes`: `5_242_880_000`
- **API token**
  - `full`: `tk_test0001abcdef` (the value tests use as `Authorization: Bearer ...`)
  - `prefix`: `tk_test0` (first 8 chars)
  - `user_id`: `u_alice`
  - `name`: `default-test-token`

No sites or connect sessions are seeded by default — tests add their own via
`/__test/seed`.

## Auth Middleware

`requireBearer` (in `src/auth.ts`) inspects the `Authorization` header and
attaches the resolved `User` + `Token` to the Hono context (`c.get('user')`,
`c.get('token')`). Missing, malformed, or unknown bearer tokens produce a
`401 auth_required` envelope. Business routes mount this middleware on
the authenticated `/api/*` group.

## Rust Harness Invocation

The crate-level harness (`crates/als/tests/common/`) spawns this process and
reads the handshake:

```rust
let mut child = Command::new("bun")
    .args(["run", "tests/mock-server/src/main.ts", "--port", "0"])
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit())
    .spawn()?;
let mut line = String::new();
BufReader::new(child.stdout.take().unwrap()).read_line(&mut line)?;
let url = line.strip_prefix("LISTENING ").unwrap().trim();
```

Each test resets state via `POST /__test/reset` before issuing requests.
See [`../../docs/development.md`](../../docs/development.md#mock-server-lifecycle)
for the full harness contract.

## Typecheck

```bash
bun run typecheck
```

Runs `tsc --noEmit` against `tsconfig.json` (strict mode, ES2022, bundler
module resolution, `@types/bun`).

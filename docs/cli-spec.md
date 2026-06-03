# CLI specification

`als` is a single static binary that uploads and publishes static sites,
markdown / mdbook books, and read-only code snippets to an `a.ls` server.
This file documents the user-facing surface. For the internal module map
see [`architecture.md`](architecture.md); for the typed client see
[`api-reference.md`](api-reference.md); for the server contract see
`cli-server-contract.md` in the als-docs repo.

## Quickstart

```bash
als auth login        # one-time device-flow login; persists a token
als ./build           # publish a folder → prints the public URL
als ./build           # re-run later: same URL, fresh version, no flags needed
als list              # list owned sites
als rm <id>           # remove a site
```

Defaults the CLI ships with (override via env, flags, or the config file):

| Setting       | Value                       |
|---------------|-----------------------------|
| API base      | `https://api.a.ls`          |
| Homepage      | `https://a.ls`              |
| LLM cheatsheet | `https://a.ls/llms.txt`    |

## Conventions

- `[OPTIONS]` in a synopsis stands for any combination of the global
  options listed below — they accept the same value whether they appear
  before or after the subcommand.
- `<arg>` is a required positional argument; `[arg]` is optional.
- `a|b` (with no brackets) means "either `a` or `b`".
- Example output is shown after `$` prompts.

### Global options

Accepted by every subcommand:

| Option              | Effect                                                          |
|---------------------|-----------------------------------------------------------------|
| `--profile <name>`  | Pick a named profile from the config file.                      |
| `--quiet`           | Decoration-free output — just the URL (or the essential line).  |
| `--json`            | Emit a machine-readable JSON envelope instead of human text.    |

## Command index

```
als [OPTIONS] <PATH>                       upload and publish (no-subcommand form)
als [OPTIONS] list                         list owned sites with quota footer
als [OPTIONS] site <id|name> [edit flags]  detail view, or partial update
als [OPTIONS] rm <id|name>                 remove a site (interactive confirm)
als [OPTIONS] rm --all-expired             remove every expired site
als [OPTIONS] unpin <path|name|id>         drop a local path → site binding
als [OPTIONS] auth login [-T tk | --no-qr] device-flow login (or paste-token)
als [OPTIONS] auth logout                  clear the local token
als [OPTIONS] auth revoke [-y]             revoke server-side, then clear locally
als [OPTIONS] auth status                  current user + quota footer
als [OPTIONS] preview <path>               local HTTP preview of a path
als [OPTIONS] config get|set <key> [val]   read / write the local config
als [OPTIONS] config list                  print the on-disk config (tokens masked)
als [OPTIONS] completion <shell>           emit a shell completion script
als help [<command>]                       printed help
```

---

## Deploy: `als <path>`

The no-subcommand form. The CLI auto-detects the input shape and picks
between two upload modes (`site` or `code`).

### Synopsis

```
als <path> [--name N] [--pass P|auto] [--expires DUR] [--kind site|code] [--new]
```

### Input shapes

| Input                                       | Mode | How it's packed |
|---------------------------------------------|------|-----------------|
| Folder with `index.html`                    | site | Recursive zip of the folder. |
| Folder with `book.toml`                     | site | `mdbook build`, then zip the rendered tree. |
| Folder with `*.md` (no `book.toml`)         | site | Auto-bootstrap mdbook, build, zip. |
| Single `*.html`                             | site | Wrap as `<stem>/index.html`, zip. |
| Single `*.md`                               | site | One-chapter mdbook build, zip. |
| `.zip` archive                              | site | Upload as-is. |
| Folder of recognised code (no html / md / book.toml) | code | Viewer shell + manifest + `raw/`, zip. |
| Single recognised code file                 | code | Same as above with one file. |

`--kind site` / `--kind code` overrides auto-detect (e.g. force the
viewer on a folder that happens to also contain an `index.html`).

### Flags

| Flag            | Default                  | Notes |
|-----------------|--------------------------|-------|
| `--name <s>`    | local pin / path-derived | Project name — the human-readable identity key (see [Project pinning](#project-pinning)). Re-deploying with the same name pushes a new version to the existing site; the URL stays put. Unique per user. Kebab-case lowercase only (`[a-z0-9-]`, no leading or trailing `-`); the CLI rejects anything else at parse time. |
| `--pass <p>`    | unchanged on update      | `auto` for a server-generated memorable password, or paste a literal one. |
| `--expires <d>` | unchanged on update      | Duration shorthand (`5m` / `24h` / `7d` / `1y`), `never`, or RFC 3339. |
| `--kind <k>`    | auto                     | `site` or `code`. |
| `--new`         | off                      | Force-create a fresh site (skips both the local pin and the server-side name dedup). Mutually exclusive with `--name`. |

### Examples

Every site has two distinct identifiers, surfaced separately in the
output:

- **Id** — 10-char base32 nanoid (`[a-z2-7]{10}`), auto-generated by
  the server, immutable, IS the URL subdomain (`<id>.a.ls`).
- **Name** — kebab-case lowercase label (`[a-z0-9-]`, no leading or
  trailing `-`). With `--name my-proj` the user picks it explicitly.
  Without, the CLI mints `<word><NNN>` — one of 64 curated short
  nouns (animals / colors / nature) concatenated with a three-digit
  number, e.g. `fox042`, `mint007`, `panda500`. Generated purely
  locally; the server is the sole arbiter of uniqueness and rejects
  any clash. Mutable via `als site <id|name> --name <new>`.

First deploy:

```
$ als ./build --name myproj
✓ Deployed: https://k7x2qm4j6p.a.ls
  Id:      k7x2qm4j6p
  Name:    myproj
  Version: 20260510-103022-a3f1c2
  Expires: in 7 days
```

Re-deploy (any input that resolves to the same name):

```
$ als ./build --name myproj
✓ Deployed: https://k7x2qm4j6p.a.ls
  Id:      k7x2qm4j6p
  Name:    myproj
  Version: 20260516-091500-b8e5d3
  Expires: in 7 days
```

Same URL (the `Id` never changes), fresh version id. The version id is
**display-only**: there is no `--version` flag on deploy. Rolling back
goes through [`als site`](#site-als-site-idname):

```
$ als site k7x2qm4j6p                  # detail view + version history
$ als site k7x2qm4j6p --version <id>   # activate a past version
```

Quiet mode (just the URL — handy in scripts):

```
$ als ./build --quiet
https://k7x2qm4j6p.a.ls
```

JSON mode (script-consumable):

```
$ als ./build --json
{"id":"k7x2qm4j6p","url":"https://k7x2qm4j6p.a.ls","project_name":"myproj","version":"20260510-103022-a3f1c2","expires_at":"2026-05-17T10:30:22Z","auth_required":false,"password":null}
```

The JSON envelope keeps the wire field name `project_name` to match
the server contract; that field is what the Human output renders as
`Name`.

### Project pinning

Re-running `als <path>` from the same checkout does **not** create a new
site. Four layered mechanisms keep a path on a stable URL:

1. **`als.toml.id` (committed, cross-machine).** Highest-priority
   anchor. Every successful deploy writes the server-assigned id back
   into `<path>/als.toml`, so the file becomes self-populating after
   the first run. Once committed, any clone of the source tree —
   fresh machine, CI runner, second laptop — deploys to the same
   site without depending on the local cache. `--new` is the only
   thing that bypasses it.
2. **`site_id` anchor from the local pin store.** When `als.toml.id`
   is absent (legacy projects, single-file uploads, or a directory
   the user hasn't yet committed `als.toml` for), the CLI falls back
   to the pin file under `~/.config/als/sites/`. Same wire effect —
   `site_id` on the request — just sourced per-machine instead of
   per-repo. Saves the cross-machine-rename case: if you rename the
   site on machine A while machine B's pin still says the old name,
   B's next deploy still resolves to the same site because the id
   overrides the stale name.
3. **Server-side name uniqueness (no-id fallback).** With no id from
   either source above, `project_name` becomes the identity key. The
   server enforces case-insensitive uniqueness per user; a hit pushes
   a new version, a miss allocates a fresh site.
4. **Local pin store as cache.** Each successful deploy writes a TOML
   file under `~/.config/als/sites/<id>_<name>.toml` recording
   `(canonical path, name, id, url, last_published, last_content_hash)`.
   That cache backs mechanism 2 and feeds the no-op-skip
   content-hash check; it is **not** consulted when `als.toml.id`
   already pins the site.

```
$ als ./build --name myproj        # first time: server allocates id k7x2qm4j6p
✓ Deployed: https://k7x2qm4j6p.a.ls

$ als ./build                      # later, same checkout: auto-detected
✓ Deployed: https://k7x2qm4j6p.a.ls   # same URL, server assigns a fresh version
```

Resolution order, highest precedence first:

| Source                                                            | Behaviour |
|-------------------------------------------------------------------|-----------|
| `--new`                                                           | Force a fresh site. Both `als.toml.id` and the pin store are ignored, no `site_id` sent. The new id is still written back to `als.toml` afterwards (overwriting any committed value), so subsequent runs pin to the new site. Mutually exclusive with `--name`. |
| `als.toml.id` in the input directory                              | Send that as `site_id` on the wire. `--name <n>` on top of this is a rename hint (id still anchors the request; the server applies the new name). Without `--name` the wire carries `site_id` alone and the server's current name wins. |
| Local pin file under `~/.config/als/sites/`                       | Same as the row above but the id comes from the per-machine cache — used for inputs without an `als.toml` (single HTML files, zips, legacy directories that haven't been deployed-then-committed yet). |
| `--name <n>` with no prior id from either source                  | Send `project_name=<n>` only; server-side name dedup decides whether to add a version or create a new site. |
| `als.toml.name` with no id and no `--name`                        | Same as the above row but the name comes from the committed `als.toml.name` — lets a fresh clone on a different machine bind to an existing site by name before the auto-written id has had a chance to land. |
| None of the above                                                 | CLI mints `<word><NNN>` — a pronounceable noun from a 64-entry curated list concatenated with a three-digit number, e.g. `fox042`. The server allocates a fresh id and the CLI auto-writes it into `als.toml` so the next deploy uses the id anchor. |

[`als unpin <key>`](#unpin-als-unpin-key) removes a local pin;
[`als rm`](#remove-als-rm) clears its own pin so a deleted site does
not silently rebind on the next deploy.

#### Pin file format

One file per site, named `<id>_<name>.toml` — `id` comes first so a
`ls` of the directory is roughly random-order but every entry is
locatable by either side of the underscore. The name is already
lowercase (the `--name` validator enforces it), so the filename
matches the stored name byte-for-byte:

```
~/.config/als/sites/
├── k7x2qm4j6p_myproj.toml
└── m3p4rs5t2k_fox042.toml
```

Contents:

```toml
name              = "myproj"            # lowercase kebab-case
id                = "k7x2qm4j6p"
path              = "/home/alice/build" # canonical absolute path
url               = "https://k7x2qm4j6p.a.ls"
last_published    = "2026-05-15T19:50:00Z"
last_content_hash = "a3f1c2..."         # SHA-256 of post-filter source bytes
```

The two-part filename gives the CLI cheap O(N) prefix / suffix
lookups without parsing TOML:

- by id → `<id>_*.toml`
- by name → `*_<name>.toml`

Editing or deleting these files by hand is fully supported —
`rm ~/.config/als/sites/k7x2qm4j6p_*.toml` is the unscripted
equivalent of `als unpin k7x2qm4j6p`. On rename (`als site --name
<new>`), the next deploy rewrites the file as `<id>_<new>.toml` and
drops the stale one.

#### No-op skip

Without any metadata flag, `als <path>` fingerprints the source content
(SHA-256 over the post-filter tree) and compares it to `last_content_hash`
in the pin. A match short-circuits the whole pipeline — no mdbook render,
no zip, no network — and exits 0:

```
$ als ./build                         # already deployed; source unchanged
· No changes since last deploy: https://k7x2qm4j6p.a.ls
  Id:      k7x2qm4j6p
  Name:    myproj
  (run with --new for a fresh site, or --pass/--expires/--name to update metadata)
```

Any of `--name`, `--new`, `--pass`, `--expires`, `--kind` flips back to
"always run" semantics because the user has signalled intent to change
something the server stores.

### Default ignores

Always stripped from the archive before upload, regardless of mode:

- `.git/`, `.svn/`, `.hg/`
- `node_modules/`, `target/`, `dist/`, `__pycache__/`
- `.DS_Store`, `Thumbs.db`
- `.env`, `*.pem`, `*.key`  *(avoid leaking secrets)*
- `als.toml` itself  *(CLI metadata, never content)*

See [`als.toml`](#alstoml) below to layer per-project patterns on top.

### `als.toml`

Optional. Place at the root of the input. The whole file is dropped from
the bundle before upload — it's CLI metadata, never content.

```toml
# Every field is optional. Unknown fields are a hard error so typos
# surface immediately (no silent ignore).

# Authoritative site id — the highest-priority pin. Committed in the
# repo, so every clone on every machine deploys to the same site
# without depending on the per-machine pin store. Auto-written by
# `als <path>` after every successful deploy, so the field is
# self-populating: you almost never set it by hand. 10-char lowercase
# base32 (`[a-z2-7]{10}`); the loader rejects anything else.
id           = "k7x2qm4j6p"

# Project name — same identity key the server enforces uniqueness on
# (`project_name` on POST /api/deploy). Falls back from `id` when no
# id has been written yet; once `id` is present, the server's current
# name is the source of truth and this field is read for display only.
# Same kebab-case rule as the `--name` flag. Overridden per-invocation
# by `--name <n>`.
name         = "fibonacci-demo"

# Viewer page title (code mode only). Falls back to the directory
# basename when not set.
title        = "Q3 Fibonacci demo"

# Which file the viewer opens on first load (code mode only).
# The path is relative to the input root and must survive the
# `exclude` filter; otherwise the CLI exits with
# `code_default_file_missing`.
default_file = "src/fibonacci.ts"

# Extra gitignore patterns layered on top of the default ignore set
# (above). Applies to every deploy mode — site, markdown, mdbook, code.
# Gitignore syntax with negation; a leading `!` re-includes a previously
# ignored entry.
exclude      = [
    "scratch/**",
    "*.log",
    "!keep-this.log",
]
```

| Field          | Type      | Default     | Modes        | Effect |
|----------------|-----------|-------------|--------------|--------|
| `id`           | string    | none        | every mode   | Pinned site id — highest-priority anchor on `POST /api/deploy`. Sent as the wire `site_id`; the server resolves identity by it and ignores `name` for the lookup. Auto-written by the deploy command on every success, so manual edits are rarely necessary. 10-char lowercase base32 (`[a-z2-7]{10}`); anything else fails loudly with `Error::Config`. Bypassed only by `--new`. |
| `name`         | string    | none        | every mode   | Project identity used when no `id` is available yet. Sent as `project_name`; the server's per-user uniqueness key. Validated against the same kebab-case rule as `--name` (`[a-z0-9]([a-z0-9-]*[a-z0-9])?`); a bad value fails loudly with `Error::Config`. Overridden by `--name <n>` on the CLI. |
| `title`        | string    | folder name | code only    | Viewer page title (`<title>` + `data-title` on the generated shell). |
| `default_file` | string    | auto-picked | code only    | Viewer opens this file on load; must exist in the bundle. Without it the picker is `main.{ext}` / `index.{ext}` / `app.{ext}` at the top level, falling through to the first file alphabetically. |
| `exclude`      | string[]  | `[]`        | every mode   | Extra gitignore patterns. Single source of truth for per-project filtering — there is no separate ignore file. Negation (`!pattern`) can re-include defaults like `target/` if you really need to. |

The file is parsed by `als-pack` (`AlsConfig`) and consulted by both
the deploy pipeline and `als preview`. Schema-validation is strict —
the CLI exits with `config: invalid TOML` / `config: unknown field` so
typos never silently change behaviour.

### Code mode

When the input is pure code, the CLI generates a small HTML shell plus a
`manifest.json` plus a `raw/` directory mirroring the source tree, then
zips the result. The deployed site loads a read-only CodeMirror 6 viewer
(`toss-code-viewer`) from jsdelivr at a pinned version — the file tree,
tabs, and syntax highlighting all render in the visitor's browser.

#### Limits

The CLI exits 1 without uploading if any fire:

| Limit                                | Threshold | Error code                  |
|--------------------------------------|-----------|-----------------------------|
| Files (after ignore filter)          | ≤ 50      | `code_too_many_files`       |
| Per-file size                        | ≤ 256 KB  | `code_file_too_large`       |
| Total size                           | ≤ 2 MB    | `code_total_too_large`      |
| Directory depth                      | ≤ 8       | `code_depth_exceeded`       |
| At least one non-binary file         | —         | `code_no_files`             |
| `--kind code` on a non-code shape    | —         | `code_unsupported_input`    |
| `als.toml.default_file` not bundled  | —         | `code_default_file_missing` |

Binary files (NUL / non-printables in the first 1024 bytes) are skipped
automatically and do not count against the limits.

See [`als.toml`](#alstoml) above for the `title` / `default_file` /
`exclude` schema — both code-only fields live there.

#### Upload vs local preview

| Path                  | viewer.js source |
|-----------------------|------------------|
| `als <code>` upload   | jsdelivr CDN: `toss-code-viewer@<VERSION>/viewer.js` (full bundle, 16 language packs). The version is pinned in the generated HTML. |
| `als preview <code>`  | The lite bundle (5 language packs) embedded into the binary via `include_str!`. Unsupported languages fall back to plain text with a status-bar hint. |

The server is oblivious — both paths POST the resulting zip to
`/api/deploy`.

---

## List: `als list`

```
$ als list
ID          NAME           URL                EXPIRES    AUTH
k7x2qm4j6p  myproj         k7x2qm4j6p.a.ls    6d 23h     none
m3p4rs5t2k  apispec        m3p4rs5t2k.a.ls    23h 50m    pass
q4f2at3v7y  acmedemoq3     q4f2at3v7y.a.ls    never      pass

3 sites (total 3)
```

### Flags

| Flag              | Effect |
|-------------------|--------|
| `--state <s>`     | `active` (default) / `expired` / `released` / `all`. |
| `--auth <type>`   | `none` / `pass` — filter by visitor-password presence. |
| `--limit <n>`     | Cap rows requested (server cap is 200). |

JSON mode emits a `{ data: [...], total: N }` envelope.

---

## Site: `als site <id|name>`

Detail view; combined with edit flags it becomes a partial update.

```
$ als site k7x2qm4j6p
✓ k7x2qm4j6p.a.ls
  Id:        k7x2qm4j6p
  Name:      apispec
  Pass:      none
  Created:   2026-05-10 10:30:22
  Expires:   2026-05-17 10:30:22 (in 6d 23h)

  Versions  (3 total, * = current)
    * 20260510-103022-a3f1c2   2026-05-10  1.2 MB
      20260509-180000-12ab34   2026-05-09  1.1 MB
      20260508-120000-99cdef   2026-05-08  1.0 MB
```

The detail view can also be addressed by project name:
`als site apispec`. Names are matched case-insensitively against the
first 200 of the caller's sites; an ambiguous name yields exit 1 with
the matching ids printed.

### Edit flags

Any combination is accepted and optimised into the smallest set of API
calls.

| Flag                 | Effect |
|----------------------|--------|
| `--name <s>`         | Rename the project (URL unchanged — only the `Name` field shifts; `Id` is immutable). Kebab-case lowercase (`[a-z0-9-]`, no leading or trailing `-`). |
| `--expires <d>`      | New expiration (`5m` / `24h` / `7d` / `1y` / `never` / RFC 3339). |
| `--version <v>`      | Activate a past version (rollback / roll-forward). |
| `--pass <p>\|auto`   | Set / replace the visitor password. |
| `--no-pass`          | Clear the visitor password (make the site public). |
| `-y`, `--yes`        | Skip the interactive confirm for destructive flags (`--no-pass`, `--version`). |

`--pass` and `--no-pass` are mutually exclusive.

---

## Remove: `als rm`

```
$ als rm k7x2qm4j6p
? Confirm delete 'k7x2qm4j6p.a.ls' (name: apispec)? [y/N] y
✓ Removed.

$ als rm --all-expired -y
✓ Removed 4 expired sites.
```

### Flags

| Flag             | Effect |
|------------------|--------|
| `-y`, `--yes`    | Skip the interactive confirm. |
| `--all-expired`  | Remove every site in `expired` state. |

The two flags are independent; `--all-expired -y` is the
non-interactive batch form.

`als rm` also drops the matching pin file from `~/.config/als/sites/`
so the next `als <path>` does not silently rebind.

---

## Unpin: `als unpin <key>`

Removes the matching `~/.config/als/sites/<id>_<name>.toml`
file without touching the server. The next `als <path>` from the
unpinned canonical path falls back to the resolution chain (`--name`,
then path lookup, then a fresh site). Hand-deleting the file has the
same effect.

```
$ als unpin ./build
✓ Unpinned /home/alice/build (was bound to k7x2qm4j6p "myproj").

$ als unpin myproj         # by project name
$ als unpin k7x2qm4j6p          # by site id
```

`<key>` matches a binding by canonical path, project name, or site id
(in that priority order). If no binding matches, the command exits 0
with `No matching pin found.`

---

## Auth: `als auth login`

Implements the **RFC 8628 OAuth 2.0 Device Authorization Grant** against
`/api/auth/device/*`. The CLI never sees OIDC directly — the user
authorises the device in any signed-in browser, which can be on a
different machine entirely (handy over SSH, in CI, or from a phone).

```
$ als auth login

To authorize this device, visit:
  https://a.ls/verify
and enter the code:
  ABCD-EFGH
Or open this one-tap link in any signed-in browser:
  https://a.ls/verify?user_code=ABCD-EFGH

  █▀▀▀▀▀█ ▄▀█▀  █▀▀▀▀▀█      (terminal QR of the one-tap URL)
  █ ███ █ ▄▄ █▀ █ ███ █
  ...

Waiting for authorization...
✓ Logged in.
```

The CLI **never spawns a browser** — the operator opens one of the
printed URLs themselves (or scans the QR). Internal sequence:

1. `POST /api/auth/device/authorize` → user-code, verification URL,
   polling interval.
2. Render: codes + plain URL + one-tap URL + (TTY only) terminal QR.
3. Poll `POST /api/auth/device/token` at the server-supplied `interval`;
   doubles on `slow_down` (cap 30 s); exits 1 with `login_denied` or
   `login_timeout` on the terminal states. On success, persist the token
   to the active profile's `config.toml` and exit 0.

### Flags

| Flag              | Effect |
|-------------------|--------|
| `--no-qr`         | Suppress the terminal QR (URLs are still printed). |
| `-T <token>`      | Paste a pre-minted token; skip the device flow entirely. CI escape hatch. |

---

## Auth: `als auth logout` / `revoke` / `status`

```
$ als auth logout
✓ Local token cleared.

$ als auth revoke
This will invalidate the token in your config.toml.
? Revoke token tk_a1b2c3...? [y/N] y
✓ Token revoked server-side and cleared locally.

$ als auth status
Profile: default
User:    Alice Liu <alice@example.com>
API:     https://api.a.ls
Token:   tk_a1b2c3** (tk_a1b2c3…)
Quota:   3 / 100 sites · 8.3 MB / 5.0 GB per-archive
```

| Subcommand     | Wire call                                            | Effect |
|----------------|------------------------------------------------------|--------|
| `auth logout`  | (local only)                                         | Removes the token from local config; the server-side token continues to exist. |
| `auth revoke`  | `DELETE /api/tokens/:prefix`                         | Invalidates the token server-side, then clears it locally. |
| `auth status`  | `GET /api/account/me` + `GET /api/quota/me`          | Render the active profile, user, API URL, masked token, quota. |

---

## Preview: `als preview <path>`

Local HTTP preview of what `als <path>` would publish. Routes the input
through the same pre-pack pipeline (`als-md` for markdown / mdbook,
`als-code` for code) and serves the result on a local port.

### Synopsis

```
als preview <path> [--bind 127.0.0.1] [--port 0] [--kind site|code]
```

| Flag              | Default       | Effect |
|-------------------|---------------|--------|
| `--bind <addr>`   | `127.0.0.1`   | Pass `0.0.0.0` to expose to the LAN. |
| `--port <n>`      | `0` (OS pick) | Pin to a specific port. |
| `--kind <k>`      | auto          | Force `site` or `code` (same semantics as the deploy flag). |

### Supported shapes

Mirrors the deploy auto-detect:

| Shape                                 | Pipeline |
|---------------------------------------|----------|
| Folder with `index.html`              | Serve as-is. |
| Folder with `book.toml` / `*.md`      | `als-md` mdbook render into a tempdir. |
| Single `.md` file                     | `als-md` single-chapter render. |
| Single `.html` file                   | Copied to `tempdir/index.html`. |
| `.zip` archive                        | Extracted into a tempdir (zip-slip safe). |
| Pure-code folder / single code file   | `als-code` bundle into a tempdir. |

```
$ als preview ./docs
Preview at http://127.0.0.1:54123/
  press Ctrl+C to stop
```

The tempdir lives for the lifetime of the preview process; SIGINT /
SIGTERM unblock the request loop and `Drop` removes the tree.

---

## Config: `als config`

```
$ als config get default.api
https://api.a.ls

$ als config set default.api https://api.a.ls
$ als config set staging.token tk_yyy

$ als config list           # print the on-disk file with tokens masked
[default]
api   = "https://api.a.ls"
token = "tk_a1b2c3**"
```

Keys are formatted `<profile>.<field>`. Fields: `api`, `token`. The
`list` subcommand prints `~/.config/als/config.toml` verbatim with the
`token` of every section masked.

---

## Completion: `als completion <shell>`

Emits a shell completion script to stdout. Supported shells: `bash`,
`zsh`, `fish`, `powershell`, `elvish`.

```
als completion bash > /etc/bash_completion.d/als
als completion zsh  > $fpath[1]/_als
als completion fish > ~/.config/fish/completions/als.fish
```

---

## Configuration

Two things live under `~/.config/als/`:

```
~/.config/als/
├── config.toml                     # API URL + bearer token, per-profile blocks
└── sites/                          # one .toml per locally-pinned site
    ├── k7x2qm4j6p_myproj.toml      # <id>_<name>.toml
    └── m3p4rs5t2k_fox042.toml
```

XDG resolution rules apply: `$XDG_CONFIG_HOME/als/...` if set, else
`~/.config/als/...` on Linux / macOS, `%APPDATA%\als\...` on Windows.

### `config.toml`

```toml
# Default profile — used when no `--profile` / ALS_PROFILE is set.
[default]
api   = "https://api.a.ls"                    # override of the built-in default
token = "tk_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" # bearer token from `als auth login`

# Optional named profiles. Select one with `--profile <name>` or
# `ALS_PROFILE=<name>`. Same schema as `[default]`.
[profile.staging]
api   = "https://api-staging.a.ls"
token = "tk_yyyyyyyy"

[profile.work]
api   = "https://api.a.ls"
token = "tk_zzzzzzzz"
```

Schema, one block per profile:

| Section            | Field   | Type   | Required? | Notes |
|--------------------|---------|--------|-----------|-------|
| `[default]`        | `api`   | string | no        | Override of the built-in `https://api.a.ls`. Must be a valid `http://` / `https://` URL. |
| `[default]`        | `token` | string | yes (after login) | Bearer token. Read into `secrecy::SecretString`; `Debug` prints `[REDACTED]`. |
| `[profile.<name>]` | `api`   | string | no        | Same semantics as `[default].api`. |
| `[profile.<name>]` | `token` | string | yes       | Same semantics as `[default].token`. |

File mode is `0600` on Unix (the CLI's atomic-rename save enforces it).
Tokens are persisted **in plain text** — protect the file like any
other secret (do not back it up to public storage). `als config list`
prints the on-disk file with tokens masked to `<first-8-chars>**`.

#### Resolution order

For each of `api`, `token`, and the active profile:

1. **Environment variable** — `ALS_API` / `ALS_TOKEN` / `ALS_PROFILE`
   (always wins).
2. **Selected profile section** — `[profile.<name>]` when a profile is
   active, else `[default]`.
3. **Built-in default** — only `api` has one (`https://api.a.ls`);
   missing `token` errors with `missing API token`.

Environment overrides:

| Variable         | Effect                                            |
|------------------|---------------------------------------------------|
| `ALS_API`       | Override the active profile's API URL.            |
| `ALS_TOKEN`     | Override the active profile's bearer token.       |
| `ALS_PROFILE`   | Select a named profile.                           |
| `ALS_TRANSPORT` | Force a transport for this invocation: `h3` / `h2` / `auto`. Unset = consult `[transport.<host:port>]` in `config.toml` (see below). Never written back. |

#### Transport pin: `[transport.<host:port>]`

Optional user-managed table that forces a transport for a given API
server. The CLI **never writes** this section — pinning is something
the user opts into by editing `config.toml`.

```toml
[transport."api.a.ls:443"]
protocol = "h3"
```

| `protocol`     | Meaning                                                                              |
|----------------|--------------------------------------------------------------------------------------|
| `"h3"`         | **Force** HTTP/3. No probe, no fallback. Network errors are surfaced to the caller.  |
| `"h2"`         | **Force** HTTP/1.1+2. No probe.                                                      |
| `"auto"`       | Probe HTTP/3 once per CLI invocation; fall back to HTTP/2 in-memory if QUIC fails. Never written back. |
| (missing)      | Same as `"auto"`.                                                                    |

The host key is `<host>:<port>` so `https://api.a.ls`, `https://api.a.ls/api/`,
and `https://api.a.ls/anything` all share one entry.

`ALS_TRANSPORT=h3|h2|auto` overrides any file entry for the current
invocation only; it is runtime-only.

Independence from `[default]` / `[profile.*]`: the transport section
has its own lifecycle. `als auth login` / `logout` / `revoke` only
touch the `token` field of the active profile; transport pins survive
those operations untouched.

### `sites/<id>_<name>.toml`

One file per locally-pinned site (see [Pin file format](#pin-file-format)
above). The CLI writes these on every successful deploy and removes
them on `als unpin` / `als rm <id|name>`. Editing or deleting them by
hand is supported.

The directory keeps each binding isolated, so `rm`, `cat`, `grep`,
backup tooling, etc. all work on a per-site granularity. The CLI uses
filename pattern matching for cheap lookups:

- by id   → `<id>_*.toml`
- by name → `*_<name>.toml`
- by path → load every file in the directory and match on the parsed `path` field

---

## Output modes

Set via the global flags:

| Mode    | Selector    | Behaviour |
|---------|-------------|-----------|
| Human   | (default)   | Coloured + decorated text. ANSI auto-strips on pipes (`anstream`). |
| Quiet   | `--quiet`   | Decoration-free; emits only the essential line (e.g. the URL). |
| JSON    | `--json`    | Machine-readable JSON envelope. |

---

## Exit codes

| Code | Meaning           | Triggers |
|------|-------------------|----------|
| `0`  | Success           | — |
| `1`  | General error     | Catch-all + most sentinel codes (`login_denied`, `login_timeout`, `archive_too_large`, `code_*`, …). |
| `2`  | Usage error       | clap parse failures (unknown flag, missing required arg). |
| `3`  | Authentication    | 401 / token invalid. |
| `4`  | Network           | DNS / connect / read timeouts. |
| `5`  | Server error      | 5xx responses. |
| `6`  | Quota exceeded    | 402 `quota_exceeded`. |
| `7`  | Rate limited      | 429. |

JSON mode preserves the same exit codes; the failure body is emitted as
`{"error":{"code":"<sentinel>","message":"..."}}` on stderr (stdout
stays clean).

### Error reporting

Errors with actionable hints render like:

```
$ als ./oversized
✗ archive_too_large: 78 MB exceeds 50 MB limit
  Hint: split into smaller folders.
        Console quotas: https://a.ls/settings
```

The `code_*` family (code-mode limit violations) renders similar hints —
see the [code-mode limits](#limits) table above.

---

## Offline commands

The following commands never touch the network:

- `als config get|set|list`
- `als completion <shell>`
- `als preview <path>`
- `als help`
- `als --version`

Everything else requires a reachable `api` URL in the active profile.

# sysenv

A cross-platform (**Windows / Ubuntu**) **system PATH & environment variable manager** in a single binary with subcommand-based features:

- **PATH management**: add / remove / list / check system PATH entries, persisted automatically and applied to the current environment
- **Registry import / export**: dump PATH to a Windows Registry (`.reg`), JSON or text file; import back by merge or full replace
- **Link into PATH**: put any file into a PATH directory so it can be run from anywhere
- **Environment variables**: read / set / unset / list system variables, optionally persisted or session-only
- **AI model lookup**: query model & provider information from models.dev by name
- **httpie-compatible HTTP client**: flags follow httpie conventions
- **Shortcut shims**: install `spath / senv / slink / shttp / sai` short commands with one command

```
sysenv 0.2.1

Usage: sysenv <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query the models.dev database of AI models & providers
  short  Install shell shims for every subcommand (spath/senv/slink/shttp/sai)
```

---

## Feature overview

| Feature | Subcommand | Highlights |
| --- | --- | --- |
| PATH CRUD | `path` | persisted + effective in the current session; absolute-path normalization and case-insensitive dedup; prepend option; `user`/`machine` scopes; `--temporary` mode |
| Registry import/export | `path export/import` | `.reg` / JSON / TXT formats; merge or `--replace` full replace; machine migration & backup |
| Link into PATH | `link` | hard link → symlink → copy fallback chain; Windows `.cmd` shim; custom command name; system or managed directory |
| Environment variables | `env` | get / set / unset / list; persisted by default; `--temporary` for the current shell only; `machine` scope |
| AI model lookup | `ai` | 226 providers, 8000+ models; 24 h local cache; canonical-entry preference; `--search / --list / --json / --refresh` |
| HTTP client | `http` | httpie-compatible flag subset; JSON / form / multipart / raw body; nested JSON; download / redirect / auth / offline |
| Shortcut shims | `short` | installs five short commands at once; Windows `.cmd` / Linux sh scripts; auto PATH registration |

---

## Building

Requires Rust 1.79+ (`rustup` / `cargo`).

```
cargo build --release
# artifact: target/release/sysenv(.exe)
```

### Release artifacts

- Release binaries are **UPX-compressed** and placed in `dist/`
- File names carry platform + architecture + version: `sysenv-<platform>-<arch>_v<version>`
  - Windows: `sysenv-windows-x86_64_v0.2.1.exe`
  - Ubuntu: `sysenv-linux-x86_64_v0.2.1`
- Compression example (Windows):
  ```
  upx --best -o dist/sysenv-windows-x86_64_v0.2.1.exe target/release/sysenv.exe
  ```
- Every version bump with its added / fixed features is recorded in `patch.md`

## Platform & persistence model

- **Windows**: writes the user-scope registry key `HKCU\Environment` by default (inherited by every new process); `--scope machine` writes `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment` (administrator required).
- **Ubuntu**: writes managed files under `~/.config/sysenv/` and auto-injects them into shell startup scripts so persisted variables/PATH take effect in new shells; the `machine` scope writes `/etc/environment` (root required).
- On both platforms, `path add` and `env set` also **update the current process environment** (export snippet for the parent shell on Linux, registry broadcast on Windows) — both persistence and immediate effect are satisfied.
- All write operations persist by default; adding `--temporary` prints a paste-ready shell snippet (`export` / `$env:`) instead of writing anything.

---

## 1. PATH management (`path`)

### Features

- `list`: show the full effective PATH (current process + persisted scopes merged)
- `add`: append to the end; `-p/--prepend` inserts at the front; normalizes to absolute paths, deduplicates case-insensitively, skips empty entries
- `remove`: remove a directory from any scope by absolute path
- `has`: check both the current process and persisted scopes, report whether the directory is on PATH
- `export / import`: bidirectional conversion between Registry (`.reg`), JSON (`.json`) and plain text (`.txt`)
- `--scope user|machine`: choose the write scope
- `--temporary`: no persistence, only print a paste-ready PATH snippet

### Examples

```
sysenv path list                      # list the effective PATH
sysenv path add D:\tools              # append (normalized, deduped)
sysenv path add D:\tools -p           # prepend
sysenv path add /opt/bin --scope machine   # machine scope (admin/root)
sysenv path add D:\tools --temporary # current session only (print snippet)
sysenv path remove D:\tools
sysenv path has D:\tools              # checks process + persisted scopes
```

## 2. PATH import / export (`path export/import`)

### Features

- `export` picks the format from the file extension: `.reg` (Windows Registry Editor format, double-click to import), `.json` (structured, scope-aware), `.txt` (one entry per line)
- `import` **merges** by default (dedup on absolute path, then append); `--replace` replaces the whole PATH
- Ideal for machine migration, backup/restore, and snapshots before batch edits

### Examples

```
sysenv path export backup.reg         # Windows Registry Editor format, double-click to import
sysenv path export paths.json
sysenv path import backup.reg         # merge into user PATH (deduped)
sysenv path import backup.reg --replace   # full replace
sysenv path export paths.txt          # exchange/backup on Ubuntu
```

## 3. Link into PATH (`link`)

### Features

- Put a file (from any path) into a **PATH directory**, then run it by bare name from anywhere
- Link method auto-negotiation: **hard link → symlink → copy** (each step falls back automatically on cross-volume / permission issues)
- On Windows, non-`.exe` targets (`.py`, `.cmd`, scripts…) get a generated same-name `.cmd` shim so cmd and PowerShell both invoke them
- `--name` customizes the command name; `--method hard|symlink|copy` forces a method
- `--dir` selects the install directory; `--system` uses the system directory (Windows `System32` / Linux `/usr/local/bin`)
- Managed directory defaults: Windows `%USERPROFILE%\.sysenv\bin`, Ubuntu `~/.local/bin`; if it is not on PATH it is persisted automatically (`--temporary` prints a snippet instead)

### Examples

```
sysenv link mytool.exe                # into the managed bin dir; auto PATH registration
sysenv link ./script.py               # Windows generates script.cmd shim
sysenv link tool --name t             # rename the command
sysenv link tool --method copy        # force copy (auto fallback across drives)
sysenv link tool --system             # system directory
sysenv link tool -f                   # overwrite an existing link
```

## 4. Environment variables (`env`)

### Features

- `get NAME`: read the effective value (including the inheritance chain)
- `set NAME VALUE`: **persisted** by default and applied to the current process; `--temporary` prints an `export` / `$env:` snippet; `--scope machine` writes the machine level
- `unset NAME`: removes the persisted definition and drops it from the current process
- `list`: lists all persisted variables; formats interoperate with import formats
- Values containing spaces or special characters are shell-escaped automatically so snippets can be pasted directly

### Examples

```
sysenv env get FOO
sysenv env set FOO bar                # persisted (default)
sysenv env set FOO bar --temporary    # current shell only (print export / $env: snippet)
sysenv env set FOO bar --scope machine
sysenv env unset FOO
sysenv env list
```

## 5. AI model / provider lookup (`ai`)

### Features

- Data source: the official public data of [models.dev](https://models.dev), `https://models.dev/api.json` (~5.3 MB, **226 providers, 8000+ models**); the `/models/` and `/providers/` pages render from this same data
- **24 h local cache**: Windows `%LOCALAPPDATA%\sysenv\`, Linux `$XDG_CACHE_HOME` or `~/.cache/sysenv/`; `--refresh` forces a re-fetch
- Matching: case-insensitive exact match on model `id` / `canonical_model_id` / `name`, with substring fallback
- **Canonical preference**: when several providers expose the same model name, the primary entry is chosen by majority vote on the `canonical_model_id` provider prefix (e.g. `gpt-4.1` → OpenAI), its detail is printed and the remaining providers are summarized; full ids (`openai/gpt-4.1-mini`) pin down one entry exactly
- `-s/--search`: grep-style list output (`id<TAB>name<TAB>provider` per line), `--limit` controls the count (default 20)
- `--list`: paginated browsing of all models / providers
- `-o/--output-format json|csv`: machine-readable output — `json` prints a **JSON array** (equivalent to `--json`; a single hit is still an array), `csv` prints a header + CSV table (24 columns for models, 6 for providers, RFC-4180 escaping). Works for detail, search, list and multi-match modes alike
- Model detail fields: `id / name / provider / family / description / modalities / context / output limit / cost (input/output/cache_read per 1M tokens, in USD) / reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` and more

### Examples

```
sysenv ai model gpt-4.1                  # exact lookup; picks the canonical entry when several providers match
sysenv ai model openai/gpt-4.1-mini      # full id pins one entry
sysenv ai model -s qwen --limit 10       # substring search (grep-style list)
sysenv ai model --list --limit 5         # list models (8000+ total)
sysenv ai model gpt-4.1 --json           # JSON array output (same as -o json)
sysenv ai model gpt-4.1 -o csv           # CSV table output (24 columns)
sysenv ai model -s qwen -o csv           # CSV output for search results
sysenv ai provider openai -o csv         # provider CSV (6 columns)
sysenv ai model gpt-4.1 --refresh        # force re-fetch
sysenv ai provider openai                # provider detail (api / npm / model count & list)
sysenv ai provider -s groq               # provider substring search
sysenv ai provider --list                # list all providers
```

CSV columns: models `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`; providers `id,name,api,env,npm,models_count`.

## 6. httpie-compatible HTTP client (`http`)

Flags follow [httpie](https://httpie.io) (subset).

### Features

- Method auto-detection: GET when the URL carries no method; automatic POST when data items are present
- JSON by default: `key=value` items build a JSON object; `-f/--form` switches to forms, `--multipart` for file uploads
- Full request-item syntax (httpie-compatible): data fields, raw JSON values, query parameters, request headers, multipart files, `@file` raw bodies, nested JSON construction (`a[b][c]=v`, `a[]=v`, `a[1]=v`)
- Output control: `-p/--print` (any combination of `BHbh`), `-h` headers only, `-b` body only, `-m` status line, `-v` everything; terminal default `hb`, pipes/redirection default to body only
- `-o/--output` saves the body to a file (the rest goes to stderr), `-d/--download` wget-style download
- Auth: `-a user:pass` Basic, `-A bearer -a TOKEN` Bearer token
- Network behavior: `-F/--follow` redirects, `--max-redirects`, `--timeout`, `--proxy`, `--verify no` skips TLS verification, `--offline` builds & prints the request without sending, `-I/--ignore-stdin`
- `--check-status`: exit code 3 for 3xx, 4 for 4xx, 5 for 5xx (script-friendly)
- stdin piped as the raw request body; `--raw` sets an explicit raw body
- Terminal pretty-printing: JSON responses are indented by default; `--pretty none` disables it

### Examples

```
sysenv http pie.dev/get                       # GET (terminal shows status line + headers + body)
sysenv http pie.dev/post name=John age:=29    # auto POST without method; JSON by default
sysenv http -f POST pie.dev/post name='John Smith'   # form
sysenv http -v pie.dev/get                    # full request & response
sysenv http -h pie.dev/get                    # response headers only
sysenv http GET pie.dev/get q==httpie per_page==1   # query parameters
sysenv http pie.dev/post X-API-Token:123 name=John  # request header + JSON field
sysenv http -d pie.dev/image.png              # wget-style download
sysenv http -o out.json pie.dev/get           # body to file (rest to stderr)
sysenv http POST pie.dev/post @data.json      # file as raw request body
sysenv http pie.dev/post cv@resume.pdf        # multipart file upload
sysenv http -a user:pass pie.dev/anything     # Basic auth
sysenv http -A bearer -a TOKEN pie.dev/anything   # Bearer token
sysenv http -F --max-redirects 5 pie.dev/     # follow redirects
sysenv http --check-status pie.dev/404        # exit code 4
sysenv http --offline pie.dev/post a=1        # build & print the request only
sysenv http POST pie.dev/post --raw '{"a":1}' # explicit raw body
sysenv http pie.dev/post -- -name=foo         # field names starting with - go after --
echo '{"a":1}' | sysenv http POST pie.dev/post  # stdin as raw body
sysenv http --verify no https://self-signed.example  # skip certificate verification
```

> PowerShell note: PS 5.1 strips embedded double quotes from native-program arguments; for quoted JSON / raw bodies use `\"` escaping, single quotes, or run in cmd/bash.

### Request item syntax

| Item | Meaning |
| --- | --- |
| `key=value` | data field (JSON by default, form with `-f`) |
| `key:=json` | raw JSON value (number / boolean / object / array) |
| `key==value` | URL query parameter |
| `key:value` | request header (`key:` empty = unset default header, `key;` = send empty header) |
| `key@file` | multipart file upload (`;type=mime` sets the type) |
| `key=@file` / `key:=@file` / `key==@file` / `key:@file` | read field / JSON / query / header value from a file |
| `@file` | raw request body (or `--raw` / stdin) |
| `a[b][c]=v`, `a[]=v`, `a[1]=v`, `[]:=1` | nested JSON construction |

### Supported flags at a glance

`-j/--json` `-f/--form` `--multipart` `--raw` `-p/--print` `-h/--headers` `-b/--body`
`-m/--meta` `-v/--verbose` `-o/--output` `-d/--download` `-q/--quiet` `--pretty`
`-a/--auth` `-A/--auth-type` `--proxy` `-F/--follow` `--max-redirects` `--timeout`
`--check-status` `--offline` `--verify` `-I/--ignore-stdin` `--default-scheme`

> Note: inside the `http` subcommand, `-h` means httpie-style "print response headers only", so use `sysenv help http` for help.

## 7. Shortcut shims (`short`)

### Features

- Installs short shims for **all subcommands** with one command, usable from any directory afterwards
- Windows generates `.cmd` shims (`@echo off` + forwarding), Ubuntu generates executable `sh` scripts (`chmod 755`); both forward every argument
- Installs into the managed bin directory (shared with `link`) by default and **persists it into PATH automatically**; `--temporary` only prints a snippet
- `--dir` selects the directory, `-f/--force` overwrites existing shims
- Shims forward `%*` / `"$@"` — unlimited argument count, safe with special characters

| Short command | Equivalent |
| --- | --- |
| `spath` | `sysenv path` |
| `senv` | `sysenv env` |
| `slink` | `sysenv link` |
| `shttp` | `sysenv http` |
| `sai` | `sysenv ai` |

### Examples

```
sysenv short              # install all 5 shims into the managed dir and register PATH
sysenv short --dir ~/bin  # custom directory
sysenv short -f           # overwrite existing shims
sysenv short --temporary  # no PATH persistence, print a paste-ready snippet
```

---

## Limitations

- `http`: `--auth-type digest`, `--session`, `--stream`, `--ssl`, `--cert`, custom `--boundary` are not implemented (see `sysenv help http`)
- `path export` `.reg` files are Windows-only (Registry Editor format); use `.txt` / `.json` for backups on Linux
- `link` symlinks on Windows require Developer Mode or admin rights; falls back to copy automatically
- `ai` uses models.dev's public endpoint; offline it serves the 24 h cache, and reports an error when the cache is stale and there is no network

## Development & verification

- `cargo test`: 26 unit tests on Windows, 23 on Linux (platform-specific cases gated by `cfg`)
- `cargo check`, `cargo test`, `cargo build --release` and end-to-end smoke tests have been run on both Windows (msvc) and Ubuntu (real WSL)
- End-to-end coverage: PATH CRUD & registry restore, link fallback chain, env persistence/temporary mode, full http flag smoke tests, live models.dev fetch with canonical preference, and actual execution of the five shims
- `dev/` ships local smoke tooling: `test_server.ps1` (HttpListener test server on port 18899), `smoke_http.ps1` / `smoke_http2.ps1`, `test_body.json`

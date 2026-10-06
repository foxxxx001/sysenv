# sysenv
**English** | [简体中文](./README.md)
A cross-platform (Windows / Ubuntu) system Http client & AI model lookup & PATH & environment manager in a single binary 

- **PATH management**: add / remove / list / check system PATH entries, persisted automatically and applied to the current environment
- **Registry import / export**: dump PATH to a Windows Registry (`.reg`), JSON or text file; import back by merge or full replace
- **Link into PATH**: put any file into a PATH directory so it can be run from anywhere
- **Environment variables**: read / set / unset / list system variables, optionally persisted or session-only
- **AI model lookup**: query model & provider information from models.dev by name
- **Web search**: `sysenv search` runs web searches through the Bocha AI API (`api.bochaai.com/v1/web-search`) with official request parameters
- **httpie-compatible HTTP client**: flags follow httpie conventions
- **Shortcut shims**: install `spath / senv / slink / shttp / sai / ssearch / stask` short commands with one command

```
sysenv 0.4.12 (Made by Gary-china)

Usage: sysenv <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query the models.dev database of AI models & providers
  search Web search via the configured Bocha AI API (api.bochaai.com/v1/web-search)
  task   Query and kill processes (list PID/name/path; kill by PID or name)
  short  Install shell shims for every subcommand (spath/senv/slink/shttp/sai/ssearch/stask)
```

---

## Feature overview

| Feature | Subcommand | Highlights |
| --- | --- | --- |
| PATH CRUD | `path` | persisted + effective in the current session; absolute-path normalization and case-insensitive dedup; prepend option; `user`/`machine` scopes; `--temporary` mode |
| Registry import/export | `path export/import` | `.reg` / JSON / TXT formats; merge or `--replace` full replace; machine migration & backup |
| Link into PATH | `link` | hard link → symlink → copy fallback chain; Windows `.cmd` shim; custom command name; system or managed directory |
| Environment variables | `env` | get / set / unset / list; persisted by default; `--temporary` for the current shell only; `machine` scope |
| AI model lookup | `ai` | `ai model` (models.dev: 226 providers, 8000+ models; 24 h cache; canonical preference; no args lists all; `--date` / `--open` / `--search / --list / --json / --refresh`); `ai cn-model` (datalearner: ~1015 models; `--date` filters by published); `ai chat` (multi-provider weighted round-robin chat, OpenAI / Anthropic compatible); `ai task` (task-template chat) |
| Process management | `task` | no args lists all processes; list (PID / name / path, fuzzy name match); `-o` shows the process using a port; kill by PID or name; `-f` force |
| Web search | `search` | Bocha AI web search API (`POST api.bochaai.com/v1/web-search`): `--freshness` (time filter) / `--summary` (AI summaries) / `--count` (1-50) / `--page` / `--include-domains` / `--exclude-domains`, official parameters; key from the config `search` section; `--json` raw response / `--debug` request & response |
| HTTP client | `http` | httpie-compatible flag subset; JSON / form / multipart / raw body; nested JSON; download / redirect / auth / offline; `--help` reference; `--debug` prints the actual request & response (incl. headers) |
| Shortcut shims | `short` | installs seven short commands at once (spath/senv/slink/shttp/sai/ssearch/stask); Windows `.cmd` / Linux sh scripts; auto PATH registration |

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
  - Windows: `sysenv-windows-x86_64_v0.2.8.exe`
  - Ubuntu: `sysenv-linux-x86_64_v0.2.8`
- One-shot release scripts (recommended): **temp build artifacts (`target/`) are cleaned automatically after every successful build**, only the `dist/` deliverables remain
  - Windows: `powershell -File build.ps1` (build → UPX → smoke → auto cleanup)
  - Ubuntu: `./build.sh` (same flow; downloads a static UPX when the system has none)
- Manual flow (Windows example):
  ```
  cargo build --release
  upx --best -o dist/sysenv-windows-x86_64_v0.2.8.exe target/release/sysenv.exe
  cargo clean
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
- `-s/--search`: substring-search list output; `--limit` controls the count (default 20)
- `--list`: paginated browsing of all models / providers
- **No args lists all**: `sysenv ai model` with no arguments lists every model (`--limit N` still caps the output)
- **Text lists have a header row**: the default output of no-arg / `--list` / `--search` is an aligned table headed `ID  NAME  FAMILY  LAST UPDATED` (id / name / family / last updated)
- `--date YYYY-MM-DD`: show only models whose `last_updated` is **strictly after** the given date; combines with no-arg / `--list` / `--search` / name lookups (the date filter is applied before matching); a malformed date is rejected with an error
- `--open`: show only models with `open_weights: yes`; combines with `--date` and the other filters (date first, then open weights)
- `-o/--output-format json|csv`: machine-readable output — `json` prints a **JSON array** (equivalent to `--json`; a single hit is still an array), `csv` prints a header + CSV table (24 columns for models, 6 for providers, RFC-4180 escaping). Works for detail, search, list and multi-match modes alike
- Model detail fields: `id / name / provider / family / description / modalities / context / output limit / cost (input/output/cache_read per 1M tokens, in USD) / reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` and more

### Examples

```
sysenv ai model                          # no args: list every model (same as `sai model`)
sysenv ai model --date 2026-10-01        # only models last updated after 2026-10-01
sysenv ai model --date 2026-10-01 --limit 10   # above + first 10 entries
sysenv ai model --open --limit 10        # only models with open_weights: yes
sysenv ai model --open --date 2025-01-01 # combined filters: updated after 2025 and open weights
sysenv ai model gpt-4.1                  # exact lookup; picks the canonical entry when several providers match
sysenv ai model openai/gpt-4.1-mini      # full id pins one entry
sysenv ai model -s qwen --limit 10       # substring search (grep-style list)
sysenv ai model --list --limit 5         # list models (8000+ total)
sysenv ai model gpt-4.1 --json           # JSON array output (same as -o json)
sysenv ai model gpt-4.1 -o csv           # CSV table output (24 columns)
sysenv ai model -s qwen -o csv           # CSV output for search results
sysenv ai provider openai -o csv         # provider CSV (6 columns)
sysenv ai model gpt-4.1 --refresh        # force re-fetch (--refresh lives on `ai`; `ai --refresh model gpt-4.1` also works)
sysenv ai provider openai                # provider detail (api / npm / model count & list)
sysenv ai provider -s groq               # provider substring search
sysenv ai provider --list                # list all providers
```

CSV columns: models `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`; providers `id,name,api,env,npm,models_count`.

#### 5.1 China AI model lookup (`ai cn-model`)

Data source: the [datalearner](https://www.datalearner.com/ai-models/pretrained-models) pretrained-model list (server-rendered HTML, about **1015 models**, fetched page by page and deduplicated by slug), fully independent from `ai model` (models.dev).

- Query-only: exact lookup by **name** and `-s/--search` substring search (aliases match too); there is **no** `--list` and **no** `--open` (this source does not expose open-weights info)
- `--date YYYY-MM-DD`: filters by the **published** date shown on each card, keeping only models **strictly after** the date; used alone (without a name / search term) it lists every model published after the date
- Fields: `id (slug) / name / provider (publisher) / aliases / type (精选/预览版/开源模型/闭源模型 badges) / category (e.g. 推理大模型) / published / url`
- 24 h local cache (same cache dir as `ai model`, file `datalearner-models.json`); `--refresh` forces a re-fetch
- `--limit N` caps the list (default 20); `--json` / `-o json` print a JSON array; `-o csv` prints an 8-column CSV (`id,name,provider,aliases,type,category,published,url`)

```
sysenv ai cn-model gpt-6-1-sol            # exact lookup (case-insensitive)
sysenv ai cn-model -s ernie --limit 10    # substring search (aliases match too)
sysenv ai cn-model --date 2026-09-28      # list every model published after that date
sysenv ai cn-model -s qwen --date 2026-01-01   # search + date filter
sysenv ai cn-model gpt-6-1-sol --json     # JSON array output
sysenv ai cn-model -s ernie -o csv        # CSV output
sysenv ai cn-model gpt-6-1-sol --refresh  # force re-fetch (--refresh lives on `ai`)
```

#### 5.2 AI chat (`ai chat`)

Chats with the configured providers using the OpenAI `/v1/chat/completions` or the Anthropic Messages API standard.

- Config file defaults to **`~/.sysenv/config.yaml`** (a clear message is shown when missing); `-c/--config FILE` overrides it. The format is documented in `doc/config.yaml`; the sanitized template is `doc/config.example.yaml`
- `clients` holds the providers: required `name / api_base / api_key / models` (each `models` entry has `name` plus optional `weight`, default 1, and optional `max_tokens`); `type` is `openai` (default; `open` is accepted) or `anthropic`, and the request follows the OpenAI or the Claude API standard accordingly
- Top-level `model` selects the model:
  - missing → the first model of the first provider
  - `provider:model` → the model whose provider name and model name both match
  - `provider:*` → **all** models of that provider, picked by **weighted round-robin** on `weight` (default 1)
  - `model` (bare name) → every matching model across all providers, again weighted round-robin
- The rotation state persists in `chat_state.json` next to the config, so successive calls keep rotating
- Message source: command-line arguments (joined with spaces); with no arguments, stdin is read when it is not a terminal
- Top-level `stream: true` enables SSE streaming by default (printed token by token); `--no-stream` disables it; `--debug` forces non-streaming
- `--debug`: prints the **actual HTTP request** (method / URL / headers / body) and **response** (status / headers / body) to stderr without polluting stdout

```
sysenv ai chat "hi"                       # chat with the default config (~/.sysenv/config.yaml)
sysenv ai chat hi there                   # multiple args are joined
echo "summarize this" | sysenv ai chat    # pipe via stdin
sysenv ai chat "hi" -c doc/config.yaml    # explicit config file
sysenv ai chat "hi" --no-stream           # disable streaming
sysenv ai chat "hi" --debug               # print the actual request & response (incl. headers)
```

#### 5.3 Task templates (`ai task`)

Assembles a chat message from a predefined `tasks` template, then sends it through the `ai chat` pipeline.

- **No arguments**: lists each task's `name / desc` (at most 10)
- `-t <name>`: picks the task whose `name` matches and builds the message from its `msg` (`desc` is the task description)
- `msg` placeholders `{key:default}`: pass `key:value` or `key=value` on the command line to substitute, otherwise the default is used (e.g. `{country:深圳}` + `country:北京` → 北京)
- `msg` prefixes: `file://` reads a local **relative-path** file as the message; `url:` fetches a web page as the message
- Tasks may declare a tool executed by the program (no local shell commands) and fed back to the model as real data:
  - `api: <URL template>` — fixed HTTP API: `{key}` / `{key:default}` placeholders are filled from the model tool arguments
  - `search: <source>` — built-in search source: `bilibili` / `github` / `hn` / `toutiao` / `oschina` / `smzdm` (direct data APIs), `sogou` / `enlightent` / `dongchedi` / `autohome` (aggregate search, Sogou first with Bing fallback), `szhousing` (Shenzhen housing official API), `bochaai` (Bocha AI web search API, key from the config `search` section; args: `q`/`query`, optional `freshness` / `summary` / `count` / `page` / `include_domains` / `exclude_domains`)
- `sysenv ai task --list-source` prints the full source catalogue (name / kind / purpose / URL)

```
sysenv ai task                          # list the tasks (name / desc, at most 10)
sysenv ai task -t weather               # build the message with defaults and chat
sysenv ai task -t weather country:北京   # substitute country with 北京
sysenv ai task -t weather country=北京   # '=' syntax is equivalent
```

#### 5.4 Web search (`search`)

Web search through the **Bocha AI Web Search API** (`POST https://api.bochaai.com/v1/web-search`, Bearer auth). All request parameters follow the official interface.

- **API key**: read from the top-level `search` section of the config (default `~/.sysenv/config.yaml`, `-c/--config` overrides); the first `name: bochaai` entry wins:
  ```yaml
  search:
    - name: bochaai
      key: sk-xxxxx
  ```
- **Parameters (official)**: `query` (positional, joined with spaces; stdin is read when piped); `--freshness` (`noLimit` default / `oneDay` / `oneWeek` / `oneMonth` / `oneYear` / `YYYY-MM-DD` / range); `--summary` (AI summary + per-result snippet / site / publish time); `--count` (1-50, default 10); `--page` (default 1); `--include-domains` / `--exclude-domains` (repeatable domain filters); `--json` (raw response); `--debug` (actual request & response)

```
sysenv search "2026 Nobel Prize in Physics"            # default 10 results: title + link
sysenv search Shenzhen weather today --summary --count 5
sysenv search "rust 2026" --freshness oneMonth
sysenv search "cargo tutorial" --include-domains rust-lang.org docs.rs
sysenv search "futures price" --exclude-domains baidu.com --json
echo "today's hot news" | sysenv search                # pipe via stdin
ssearch "holiday schedule"                              # ssearch shim == sysenv search
```

## 6. Process management (`task`)

### Features

- **No args lists all**: `sysenv task` (or `stask`) with no subcommand lists every process, equivalent to `task list`
- `task list [NAME]`: list every process with **PID / name / executable path**; `NAME` fuzzy-matches the process name (substring, case-insensitive), a numeric value looks up that PID
- `-o/--port <PORT>`: show only the **process using that port** (e.g. `task -o 8080` or `task list -o 8080`), combinable with the name / PID filter; Windows uses `netstat -ano`, Linux uses `ss -ltnp`
- `task kill <PID|name>`: terminate by PID or by name; a fuzzy name match kills **every** matching process; `-f/--force` forces the kill (SIGKILL on Linux, SIGTERM by default); permission failures are reported per process without aborting the rest
- Cross-platform: Windows uses a Toolhelp snapshot + `TerminateProcess`; Linux reads `/proc` and calls `kill`

### Examples

```
sysenv task                       # no args: list all processes (same as `task list`)
sysenv task -o 8080               # show the process using port 8080
sysenv task list -o 8080          # same, explicit `list` form
sysenv task list chrome           # fuzzy name match (substring, case-insensitive)
sysenv task list 1234             # look up by PID
sysenv task kill 1234             # terminate by PID
sysenv task kill notepad          # fuzzy name match; kills every match
sysenv task kill -f 1234          # force kill (SIGKILL on Linux)
```

## 7. httpie-compatible HTTP client (`http`)

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
- `--help`: prints the flag reference with examples (`-h` inside `http` keeps the httpie meaning of "response headers only")
- `--debug`: prints the **actual HTTP request** (method / URL / headers / body, incl. Content-Length) and **response** (status line / headers / body) to stderr while stdout keeps its normal output — handy for verifying what is really sent over the wire
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
sysenv http --help                            # print the flag reference with examples
sysenv http --debug pie.dev/post a=1 b:=2     # print the actual request & response (incl. headers)
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

`--debug` `--help`

> Note: inside the `http` subcommand, `-h` means httpie-style "print response headers only", so use `sysenv http --help` for help.

## 8. Shortcut shims (`short`)

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
| `stask` | `sysenv task` |

### Examples

```
sysenv short              # install all 6 shims into the managed dir and register PATH
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

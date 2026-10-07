# sys
**English** | [简体中文](./README.md)
A cross-platform (Windows / Ubuntu) system Http client & AI model lookup & PATH & environment manager in a single binary 

- **PATH management**: add / remove / list / check system PATH entries, persisted automatically and applied to the current environment
- **Registry import / export**: dump PATH to a Windows Registry (`.reg`), JSON or text file; import back by merge or full replace
- **Link into PATH**: put any file into a PATH directory so it can be run from anywhere
- **Environment variables**: read / set / unset / list system variables, optionally persisted or session-only
- **AI model lookup**: query model & provider information from models.dev by name
- **Web search**: `sys search` runs web searches through the Bocha AI API (`api.bochaai.com/v1/web-search`) with official request parameters; `--ai` switches to the AI Search API (`/v1/ai-search`) returning an AI answer and structured modal cards
- **httpie-compatible HTTP client**: flags follow httpie conventions
- **Shortcut shims**: install `spath / senv / slink / shttp / sai / ssearch / stask` short commands with one command

```
sys 0.4.13 (Made by Gary-china)

Usage: sys <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query the models.dev database of AI models & providers
  search Web search via the configured Bocha AI API (web-search; --ai uses ai-search)
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
| AI model lookup | `ai` | `ai model` (models.dev: 226 providers, 8000+ models; 24 h cache; canonical preference; no args lists all; `--date` / `--open` / `--search / --list / --json / --refresh`); `ai cn-model` (datalearner: ~1015 models; `--date` filters by published); `ai info` (local config lookup: `info provider` name/api_base/api_key, `info model` `{provider}:{name}` list, `info price` models.dev prices, `info balance` official balance via api_key, `info sale-price` official pricing page scrape); `ai chat` (multi-provider weighted round-robin chat, OpenAI / Anthropic compatible); `ai server` (`sys ai server [ADDR]` serves an OpenAI-compatible API — `/v1/chat/completions` + `/v1/models`, default 127.0.0.1:10000, bare port/IP allowed; `--help` reports the server status and prints the configured providers/models); `ai task` (task-template chat) |
| Process management | `task` | no args lists all processes; list (PID / name / path, fuzzy name match); `-o` shows the process using a port; kill by PID or name; `-f` force |
| Web search | `search` | Bocha AI web search API (`POST api.bochaai.com/v1/web-search`): `--freshness` (time filter) / `--summary` (AI summaries) / `--count` (1-50) / `--page` / `--include-domains` / `--exclude-domains`, official parameters; key from the config `search` section; `--json` raw response / `--debug` request & response. `--ai` switches to the AI Search API (`/v1/ai-search`) returning an AI answer and structured modal cards (`--no-answer` disables the AI answer) |
| Text search & replace | `file` | fd/sd-style: 1 arg searches stdin (an extension like `me.txt` displays that file; quotes force search); 2 args search a directory tree of text/source files (`-e` / `-i` / `-t` / `-w` / `-c`); 3 args OLD NEW PATH replace in place; `-S` size / `--newer` / `--older` time / `-d` depth filters; lists files when no PATTERN is given |
| Format conversion | `con` | json / csv / md / yaml interconversion: stdin by default (`cat a.json | sys con`), `-file` reads a file, `-i` input format, `-o` output format, `-out` writes a file; tables map to/from object arrays (type inference + escaping) |
| HTTP client | `http` | httpie-compatible flag subset; JSON / form / multipart / raw body; nested JSON; download / redirect / auth / offline; `--help` reference; `--debug` prints the actual request & response (incl. headers) |
| Shortcut shims | `short` | installs seven short commands at once (spath/senv/slink/shttp/sai/ssearch/stask); Windows `.cmd` / Linux sh scripts; auto PATH registration |

---

## Building

Requires Rust 1.79+ (`rustup` / `cargo`).

```
cargo build --release
# artifact: target/release/sys(.exe)
```

### Release artifacts

- Release binaries are **UPX-compressed** and placed in `dist/`
- File names carry platform + architecture + version: `sys-<platform>-<arch>_v<version>`
  - Windows: `sys-windows-x86_64_v0.2.8.exe`
  - Ubuntu: `sys-linux-x86_64_v0.2.8`
- One-shot release scripts (recommended): **temp build artifacts (`target/`) are cleaned automatically after every successful build**, only the `dist/` deliverables remain
  - Windows: `powershell -File build.ps1` (build → UPX → smoke → auto cleanup)
  - Ubuntu: `./build.sh` (same flow; downloads a static UPX when the system has none)
- Manual flow (Windows example):
  ```
  cargo build --release
  upx --best -o dist/sys-windows-x86_64_v0.2.8.exe target/release/sys.exe
  cargo clean
  ```
- Every version bump with its added / fixed features is recorded in `patch.md`

## Platform & persistence model

- **Windows**: writes the user-scope registry key `HKCU\Environment` by default (inherited by every new process); `--scope machine` writes `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment` (administrator required).
- **Ubuntu**: writes managed files under `~/.config/sys/` and auto-injects them into shell startup scripts so persisted variables/PATH take effect in new shells; the `machine` scope writes `/etc/environment` (root required).
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
sys path list                      # list the effective PATH
sys path add D:\tools              # append (normalized, deduped)
sys path add D:\tools -p           # prepend
sys path add /opt/bin --scope machine   # machine scope (admin/root)
sys path add D:\tools --temporary # current session only (print snippet)
sys path remove D:\tools
sys path has D:\tools              # checks process + persisted scopes
```

## 2. PATH import / export (`path export/import`)

### Features

- `export` picks the format from the file extension: `.reg` (Windows Registry Editor format, double-click to import), `.json` (structured, scope-aware), `.txt` (one entry per line)
- `import` **merges** by default (dedup on absolute path, then append); `--replace` replaces the whole PATH
- Ideal for machine migration, backup/restore, and snapshots before batch edits

### Examples

```
sys path export backup.reg         # Windows Registry Editor format, double-click to import
sys path export paths.json
sys path import backup.reg         # merge into user PATH (deduped)
sys path import backup.reg --replace   # full replace
sys path export paths.txt          # exchange/backup on Ubuntu
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
sys link mytool.exe                # into the managed bin dir; auto PATH registration
sys link ./script.py               # Windows generates script.cmd shim
sys link tool --name t             # rename the command
sys link tool --method copy        # force copy (auto fallback across drives)
sys link tool --system             # system directory
sys link tool -f                   # overwrite an existing link
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
sys env get FOO
sys env set FOO bar                # persisted (default)
sys env set FOO bar --temporary    # current shell only (print export / $env: snippet)
sys env set FOO bar --scope machine
sys env unset FOO
sys env list
```

## 5. AI model / provider lookup (`ai`)

### Features

- Data source: the official public data of [models.dev](https://models.dev), `https://models.dev/api.json` (~5.3 MB, **226 providers, 8000+ models**); the `/models/` and `/providers/` pages render from this same data
- **24 h local cache**: Windows `%LOCALAPPDATA%\sys\`, Linux `$XDG_CACHE_HOME` or `~/.cache/sys/`; `--refresh` forces a re-fetch
- Matching: case-insensitive exact match on model `id` / `canonical_model_id` / `name`, with substring fallback
- **Canonical preference**: when several providers expose the same model name, the primary entry is chosen by majority vote on the `canonical_model_id` provider prefix (e.g. `gpt-4.1` → OpenAI), its detail is printed and the remaining providers are summarized; full ids (`openai/gpt-4.1-mini`) pin down one entry exactly
- `-s/--search`: substring-search list output; `--limit` controls the count (default 20)
- `--list`: paginated browsing of all models / providers
- **No args lists all**: `sys ai model` with no arguments lists every model (`--limit N` still caps the output)
- **Text lists have a header row**: the default output of no-arg / `--list` / `--search` is an aligned table headed `ID  NAME  FAMILY  LAST UPDATED` (id / name / family / last updated)
- `--date YYYY-MM-DD`: show only models whose `last_updated` is **strictly after** the given date; combines with no-arg / `--list` / `--search` / name lookups (the date filter is applied before matching); a malformed date is rejected with an error
- `--open`: show only models with `open_weights: yes`; combines with `--date` and the other filters (date first, then open weights)
- `-o/--output-format json|csv`: machine-readable output — `json` prints a **JSON array** (equivalent to `--json`; a single hit is still an array), `csv` prints a header + CSV table (24 columns for models, 6 for providers, RFC-4180 escaping). Works for detail, search, list and multi-match modes alike
- Model detail fields: `id / name / provider / family / description / modalities / context / output limit / cost (input/output/cache_read per 1M tokens, in USD) / reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` and more

### Examples

```
sys ai model                          # no args: list every model (same as `sai model`)
sys ai model --date 2026-10-01        # only models last updated after 2026-10-01
sys ai model --date 2026-10-01 --limit 10   # above + first 10 entries
sys ai model --open --limit 10        # only models with open_weights: yes
sys ai model --open --date 2025-01-01 # combined filters: updated after 2025 and open weights
sys ai model gpt-4.1                  # exact lookup; picks the canonical entry when several providers match
sys ai model openai/gpt-4.1-mini      # full id pins one entry
sys ai model -s qwen --limit 10       # substring search (grep-style list)
sys ai model --list --limit 5         # list models (8000+ total)
sys ai model gpt-4.1 --json           # JSON array output (same as -o json)
sys ai model gpt-4.1 -o csv           # CSV table output (24 columns)
sys ai model -s qwen -o csv           # CSV output for search results
sys ai provider openai -o csv         # provider CSV (6 columns)
sys ai model gpt-4.1 --refresh        # force re-fetch (--refresh lives on `ai`; `ai --refresh model gpt-4.1` also works)
sys ai provider openai                # provider detail (api / npm / model count & list)
sys ai provider -s groq               # provider substring search
sys ai provider --list                # list all providers
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
sys ai cn-model gpt-6-1-sol            # exact lookup (case-insensitive)
sys ai cn-model -s ernie --limit 10    # substring search (aliases match too)
sys ai cn-model --date 2026-09-28      # list every model published after that date
sys ai cn-model -s qwen --date 2026-01-01   # search + date filter
sys ai cn-model gpt-6-1-sol --json     # JSON array output
sys ai cn-model -s ernie -o csv        # CSV output
sys ai cn-model gpt-6-1-sol --refresh  # force re-fetch (--refresh lives on `ai`)
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
sys ai chat "hi"                       # chat with the default config (~/.sysenv/config.yaml)
sys ai chat hi there                   # multiple args are joined
echo "summarize this" | sys ai chat    # pipe via stdin
sys ai chat "hi" -c doc/config.yaml    # explicit config file
sys ai chat "hi" --no-stream           # disable streaming
sys ai chat "hi" --debug               # print the actual request & response (incl. headers)
sys ai server                          # serve an OpenAI-compatible API (default 127.0.0.1:10000)
sys ai server 8080                     # bare port: host stays 127.0.0.1
sys ai server 0.0.0.0                  # bare IP: port stays 10000 (exposed to the network)
sys ai server 0.0.0.0:9000             # full address
sys ai server --help                   # server status: address/port + configured providers/models
```

- `sys ai server [ADDR]`: **serves the configured providers as an OpenAI-compatible API** (default `127.0.0.1:10000`; `-c/--config` selects the config file). Exposes `POST /v1/chat/completions` (non-streaming passthrough and SSE streaming via chunked transfer) and `GET /v1/models` (every configured model, id `{provider}:{model}`). The request `model` follows the same selector rules as chat (`provider:model` / `provider:*` / bare model name / `auto` = config default). Only `type: openai` providers are served; anthropic targets get a clear error. The address may be a bare port (`8080` → host default) or a bare IP (`0.0.0.0` → port default).
- `sys ai server --help`: **queries the server status** — probes that address (GET /v1/models, 2 s timeout) to see whether a server is already running: if so it prints the current address/port, otherwise it says `server is NOT running`; in both cases it then prints the configured info grouped by provider — one provider line (name/api_base/api_key/DOCS/CONSOLE) followed by that provider's indented model lines (`{provider}:{name}`).

#### 5.3 Task templates (`ai task`)

Assembles a chat message from a predefined `tasks` template, then sends it through the `ai chat` pipeline.

- **No arguments**: lists each task's `name / desc` (at most 10)
- `-t <name>`: picks the task whose `name` matches and builds the message from its `msg` (`desc` is the task description)
- `msg` placeholders `{key:default}`: pass `key:value` or `key=value` on the command line to substitute, otherwise the default is used (e.g. `{country:深圳}` + `country:北京` → 北京)
- `msg` prefixes: `file://` reads a local **relative-path** file as the message; `url:` fetches a web page as the message
- Tasks may declare a tool executed by the program (no local shell commands) and fed back to the model as real data:
  - `api: <URL template>` — fixed HTTP API: `{key}` / `{key:default}` placeholders are filled from the model tool arguments
  - `search: <source>` — built-in search source: `bilibili` / `github` / `hn` / `toutiao` / `oschina` / `smzdm` (direct data APIs), `sogou` / `enlightent` / `dongchedi` / `autohome` (aggregate search, Sogou first with Bing fallback), `szhousing` (Shenzhen housing official API), `bochaai` (Bocha AI web search API, key from the config `search` section; args: `q`/`query`, optional `freshness` / `summary` / `count` / `page` / `include_domains` / `exclude_domains`)
- `sys ai task --list-source` prints the full source catalogue (name / kind / purpose / URL)

```
sys ai task                          # list the tasks (name / desc, at most 10)
sys ai task -t weather               # build the message with defaults and chat
sys ai task -t weather country:北京   # substitute country with 北京
sys ai task -t weather country=北京   # '=' syntax is equivalent
```

#### 5.4 Web search (`search`)

Web search through the **Bocha AI Web Search API** (`POST https://api.bochaai.com/v1/web-search`, Bearer auth). All request parameters follow the official interface.

- **API key**: read from the top-level `search` section of the config (default `~/.sysenv/config.yaml`, `-c/--config` overrides); the first `name: bochaai` entry wins:
  ```yaml
  search:
    - name: bochaai
      key: sk-xxxxx
  ```
- **Parameters (official)**: `query` (positional, joined with spaces; stdin is read when piped); `--ai` switches to the AI Search API (`POST /v1/ai-search`) — structured modal cards plus an AI-generated answer (official `answer` param, on by default; `--no-answer` turns it off; `stream` stays off); `--freshness` (`noLimit` default / `oneDay` / `oneWeek` / `oneMonth` / `oneYear` / `YYYY-MM-DD` / range); `--summary` (AI summary + per-result snippet / site / publish time on Web Search; AI Search always shows them); `--count` (1-50, default 10); `--page` (default 1); `--include-domains` / `--exclude-domains` (repeatable domain filters); `--json` (raw response); `--debug` (actual request & response)

```
sys search "2026 Nobel Prize in Physics"            # default 10 results: title + link
sys search Shenzhen weather today --summary --count 5
sys search "rust 2026" --freshness oneMonth
sys search "cargo tutorial" --include-domains rust-lang.org docs.rs
sys search "futures price" --exclude-domains baidu.com --json
echo "today's hot news" | sys search                # pipe via stdin
sys search "Hangzhou weather" --ai                  # AI Search: AI answer + weather card + sources
sys search "housing policy" --ai --no-answer --count 5
ssearch "holiday schedule"                              # ssearch shim == sys search
```

> Note: **AI Search and Web Search are separate Bocha packages**. The key needs an AI Search package on the open platform; without it `--ai` returns 403 (`You do not have enough money or package quota`).

#### 5.5 Local config lookup (`ai info`)

Inspects the local config file (default `~/.sysenv/config.yaml`, `-c/--config` overrides) and the models.dev model prices. **With no FIELD argument it prints the provider table (name/api_base/api_key/DOCS/CONSOLE) and the full model list (`{provider}:{name}`) together.**

- `info provider [KEYWORD]` — lists every provider under `clients` with `name / api_base / api_key` plus the official `DOCS` (help docs) and `CONSOLE` URLs (matched by provider name against a built-in table; unknown providers show `-`); with KEYWORD only providers whose name contains it are kept (case-insensitive), otherwise an error lists the available names; `-o json` / `-o csv` / `--json` print a JSON array or CSV table (JSON fields `name` / `api_base` / `api_key` / `docs` / `console`)
- `info model [KEYWORD]` — lists every configured model as `{provider}:{name}`; with KEYWORD only models of providers whose name contains it are kept (`provider:model` / `provider:*` select specific models); when no provider matches, models whose name contains KEYWORD are listed instead; `-o json` / `-o csv` / `--json` print a JSON array or CSV table (JSON fields `provider` / `name`)
- `info price P1,P2,...` — model price list (input / output / cache_read per 1M tokens, `$`) for the comma-separated provider names (half- or full-width commas), matched against models.dev ids / names (24 h cache, `--refresh` forces a re-fetch)
- `info balance PROVIDER` — queries the provider's official balance with its configured `api_key`. minimax uses the official `token_plan/remains` endpoint (Token Plan remainders; without a subscription it points to the pay-as-you-go balance page); agnes uses an OpenAI-compatible billing probe (credit_grants / subscription / usage, with a console hint when no balance field is returned); alibaba-cn (Alibaba Bailian / DashScope) uses the official `models/limits` endpoint (per-model usage quotas / rate limits, which also verify the key; no API-key cash-balance endpoint exists, so it points to the Bailian console); modelscope / sensenova / bigmodel / amd / anspire expose no API-key balance endpoint, so the command prints the matching console URL. Balance lookups are always live (no cache)
- `info sale-price PROVIDER` — scrapes the provider's official pricing page and prints every model's sale price. agnes uses `wiki.agnes-ai.cn` (text / image / video models, list + current price in ¥); minimax uses `platform.minimaxi.com` (language-model input / output / cache prices, ¥ per million tokens); alibaba-cn prints the official Bailian model list & billing page; other providers print their official pricing-page URL. Pricing pages are cached 24 h, `--refresh` forces a re-fetch

```
sys ai info                              # no field: provider table + full model list
sys ai info provider                     # every configured provider (name/api_base/api_key)
sys ai info provider agnes               # only providers whose name contains agnes
sys ai info provider -o json             # every provider as a JSON array
sys ai info provider agnes -o csv        # filtered + CSV output
sys ai info provider --json              # same as -o json
sys ai info model                        # every model as provider:model
sys ai info model modelscope             # only modelscope's models
sys ai info model deepseek               # falls back to matching model names
sys ai info model agnes:3.0              # one model: agnes models whose name contains 3.0
sys ai info model modelscope:*           # every modelscope model
sys ai info price openai,anthropic       # model prices of two providers
sys ai info price "openai，deepseek"     # full-width commas work too
sys ai info balance minimax              # query MiniMax's official balance with its api_key
sys ai info balance alibaba-cn           # Alibaba Bailian: official limits + console hint
sys ai info sale-price agnes             # scrape agnes' official pricing page (all models)
```

#### 5.6 Config export (`ai config`)

Exports the local config's `clients` section into another tool's format:

* `ai config FORMAT [SELECT] [-f FILE] [-c FILE]`; `FORMAT`:
  * **`codex`** (TOML): one `[model_providers.<name>]` table per provider (`name` / `base_url` / `env_key` / `wire_api = "chat"`). codex reads keys from env vars only, so the export names them `SYS_<PROVIDER>_API_KEY` and prints `export` hints to stderr; the top-level `model` selects the first model as `provider.model`
  * **`opencode`** (JSON): one provider entry per configured provider (`npm: "@ai-sdk/openai-compatible"` + `options.baseURL` / `options.apiKey` inline + `models` map) — mergeable into `opencode.json`
  * **`litellm`** (YAML): one `model_list` entry per model; the exposed name is `{provider}:{model}` (collision-free) routing to `openai/{model}`, with `api_base` / `api_key` inline
  * **`freellmapi`** (JSON): a `customProviders` array, one `baseUrl` / `label` / `models` entry per provider (`supportsTools: true`) — mergeable into `freellmapi.config.json`
* `SELECT`: omitted = **every provider's every model**; `provider:*` = a provider's whole model list; `provider:model` = one specific model (no match → clear error)
* `-f/--file FILE` writes to a file (stdout by default); `-c/--config` selects the config file

```
sys ai config codex                      # all models as codex TOML (stdout)
sys ai config opencode -f opencode.json  # write to a file
sys ai config litellm "agnes:*"          # only agnes' models
sys ai config freellmapi "minimax:MiniMax-M2.7"  # one specific model
```

#### 5.7 Text search & replacement (`file`)

fd/sd-style: search stdin or a directory tree of text/source files, or replace a string in place. Matching runs on the Unicode char level (`-i` folds case per char, so non-ASCII text works too).

* `file PATTERN` — read stdin and print matching lines (piping: `type a.txt | sys file hello`); **an argument with an extension (e.g. `me.txt`) displays that file's content instead** (cat-style); wrapping it in quotes (`"me.txt"`) forces the search meaning
* `file PATTERN PATH` — search PATH (a file or a directory tree) over all known text and source files (txt/md/py/java/c/...), printing `path:line:content`
* `file OLD NEW PATH` — replace OLD with NEW in place under PATH, printing per-file counts and a summary

Options:

* `-e EXT` — only search files with this extension (repeatable, leading dot optional, e.g. `-e py -e md`)
* `-i` — case-insensitive matching (search and replacement)
* `-t` — only plain-text files (txt/md/log/csv/json/...), excluding source code
* `-w` — match whole words only (not preceded/followed by a letter, digit or `_`)
* `-c NUM` — show NUM lines of context around every match (`--` separates groups)

File attribute filters (fd-style, combine with search / replace / listing):

* `-S SIZE` / `--size SIZE` — only files at least SIZE bytes. A plain number is bytes; `2k`/`2m`/`2g`/`2t` (or kb/mb/gb/tb) scale by 1024, decimals allowed (`1.5m`), case-insensitive
* `--newer TIME` — only files modified at/after TIME; `--older TIME` — only files modified before TIME. Format `YYYY-MM-DD [HH:MM[:SS]]` (`T` or slash separators work too), interpreted in the local timezone
* `-d NUM` — descend at most NUM levels of subdirectories (`-d 0` = current dir only)

Giving any filter without a PATTERN enters **list mode** (one path per line, current dir by default; `-e`/`-t` still apply); with a PATTERN or OLD NEW the filters combine with string search / in-place replacement.

Traversal: hidden entries (`.` prefix) and common noise directories (`.git` / `node_modules` / `target` / `dist` / `build` / `__pycache__` ...) are skipped; binary files (NUL bytes) are skipped; replacement honors `-e` / `-t` / `-i` / `-w` too.

```
sys file hello                              # search stdin (type a.txt | sys file hello)
sys file me.txt                             # display the file me.txt
sys file "me.txt"                           # quoted: still a string search (find me.txt in stdin)
sys file hello D:\projects                  # search the D:\projects tree
sys file hello D:\projects -e py -e md      # only .py and .md
sys file HELLO D:\projects -i               # case-insensitive
sys file hello D:\projects -t               # plain text only (no source code)
sys file hello D:\projects -w -c 2          # whole words + 2 lines of context
sys file hello hi D:\projects               # replace hello -> hi in place
sys file HELLO hi D:\projects -i            # case-insensitive replacement
sys file -S 2m                              # list files >= 2 MiB in the current dir
sys file -S 5000 D:\data                    # list files >= 5000 bytes under D:\data
sys file -S 2m -e bin -d 1 D:\data          # >= 2MiB .bin files, 1 level deep
sys file --newer "2026-10-01" D:\data       # files modified at/after 2026-10-01
sys file --older "2026-10-01 12:00" D:\data # files modified before that time
sys file hello D:\data -S 1m                # search hello only in files >= 1 MiB
```

#### 5.8 Format conversion (`con`)

Interconvert json / csv / md / yaml. Reads stdin by default (piping: `cat a.json | sys con`) and prints to stdout; `-out` writes to a file instead.

* `-file F` — read from file F (stdin when omitted; a json/csv/md/yaml extension also infers the input format)
* `-i FMT` — input format `json | csv | md | yaml` (auto-detected from content when omitted)
* `-o FMT` — output format `json | csv | md | yaml` (defaults to the input format, i.e. format-only display)
* `-out F` — write the result to file F (stdout by default)

Table conversions (csv / md) map to/from arrays of objects: the CSV first row is the header; Markdown tables parse header + data rows. Cells are type-inferred (null / bool / int / float / string); commas, quotes, newlines and `|` are escaped on output (`\|`). YAML parsing falls back to a tab-tolerant parser when serde_yaml fails, so real configs with tab indentation work.

```
cat a.json | sys con                        # stdin, auto-detect, print (defaults to the input format)
sys con -file a.json -o csv                 # json -> csv (stdout)
sys con -i csv -o json < a.csv              # csv -> json
sys con -file a.yaml -o md -out out.md      # yaml -> markdown, write to out.md
sys con -file config.yaml -o json           # real config (tab indentation) -> json
type a.csv | sys con -o md                  # csv -> markdown table
```

## 6. Process management (`task`)

### Features

- **No args lists all**: `sys task` (or `stask`) with no subcommand lists every process, equivalent to `task list`
- `task list [NAME]`: list every process with **PID / name / executable path**; `NAME` fuzzy-matches the process name (substring, case-insensitive), a numeric value looks up that PID
- `-o/--port <PORT>`: show only the **process using that port** (e.g. `task -o 8080` or `task list -o 8080`), combinable with the name / PID filter; Windows uses `netstat -ano`, Linux uses `ss -ltnp`
- `task kill <PID|name>`: terminate by PID or by name; a fuzzy name match kills **every** matching process; `-f/--force` forces the kill (SIGKILL on Linux, SIGTERM by default); permission failures are reported per process without aborting the rest
- Cross-platform: Windows uses a Toolhelp snapshot + `TerminateProcess`; Linux reads `/proc` and calls `kill`

### Examples

```
sys task                       # no args: list all processes (same as `task list`)
sys task -o 8080               # show the process using port 8080
sys task list -o 8080          # same, explicit `list` form
sys task list chrome           # fuzzy name match (substring, case-insensitive)
sys task list 1234             # look up by PID
sys task kill 1234             # terminate by PID
sys task kill notepad          # fuzzy name match; kills every match
sys task kill -f 1234          # force kill (SIGKILL on Linux)
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
sys http pie.dev/get                       # GET (terminal shows status line + headers + body)
sys http pie.dev/post name=John age:=29    # auto POST without method; JSON by default
sys http -f POST pie.dev/post name='John Smith'   # form
sys http -v pie.dev/get                    # full request & response
sys http -h pie.dev/get                    # response headers only
sys http GET pie.dev/get q==httpie per_page==1   # query parameters
sys http pie.dev/post X-API-Token:123 name=John  # request header + JSON field
sys http -d pie.dev/image.png              # wget-style download
sys http -o out.json pie.dev/get           # body to file (rest to stderr)
sys http POST pie.dev/post @data.json      # file as raw request body
sys http pie.dev/post cv@resume.pdf        # multipart file upload
sys http -a user:pass pie.dev/anything     # Basic auth
sys http -A bearer -a TOKEN pie.dev/anything   # Bearer token
sys http -F --max-redirects 5 pie.dev/     # follow redirects
sys http --check-status pie.dev/404        # exit code 4
sys http --offline pie.dev/post a=1        # build & print the request only
sys http --help                            # print the flag reference with examples
sys http --debug pie.dev/post a=1 b:=2     # print the actual request & response (incl. headers)
sys http POST pie.dev/post --raw '{"a":1}' # explicit raw body
sys http pie.dev/post -- -name=foo         # field names starting with - go after --
echo '{"a":1}' | sys http POST pie.dev/post  # stdin as raw body
sys http --verify no https://self-signed.example  # skip certificate verification
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

> Note: inside the `http` subcommand, `-h` means httpie-style "print response headers only", so use `sys http --help` for help.

## 8. Shortcut shims (`short`)

### Features

- Installs short shims for **all subcommands** with one command, usable from any directory afterwards
- Windows generates `.cmd` shims (`@echo off` + forwarding), Ubuntu generates executable `sh` scripts (`chmod 755`); both forward every argument
- Installs into the managed bin directory (shared with `link`) by default and **persists it into PATH automatically**; `--temporary` only prints a snippet
- `--dir` selects the directory, `-f/--force` overwrites existing shims
- Shims forward `%*` / `"$@"` — unlimited argument count, safe with special characters

| Short command | Equivalent |
| --- | --- |
| `spath` | `sys path` |
| `senv` | `sys env` |
| `slink` | `sys link` |
| `shttp` | `sys http` |
| `sai` | `sys ai` |
| `stask` | `sys task` |

### Examples

```
sys short              # install all 6 shims into the managed dir and register PATH
sys short --dir ~/bin  # custom directory
sys short -f           # overwrite existing shims
sys short --temporary  # no PATH persistence, print a paste-ready snippet
```

---

## Limitations

- `http`: `--auth-type digest`, `--session`, `--stream`, `--ssl`, `--cert`, custom `--boundary` are not implemented (see `sys help http`)
- `path export` `.reg` files are Windows-only (Registry Editor format); use `.txt` / `.json` for backups on Linux
- `link` symlinks on Windows require Developer Mode or admin rights; falls back to copy automatically
- `ai` uses models.dev's public endpoint; offline it serves the 24 h cache, and reports an error when the cache is stale and there is no network

## Development & verification

- `cargo test`: 26 unit tests on Windows, 23 on Linux (platform-specific cases gated by `cfg`)
- `cargo check`, `cargo test`, `cargo build --release` and end-to-end smoke tests have been run on both Windows (msvc) and Ubuntu (real WSL)
- End-to-end coverage: PATH CRUD & registry restore, link fallback chain, env persistence/temporary mode, full http flag smoke tests, live models.dev fetch with canonical preference, and actual execution of the five shims
- `dev/` ships local smoke tooling: `test_server.ps1` (HttpListener test server on port 18899), `smoke_http.ps1` / `smoke_http2.ps1`, `test_body.json`

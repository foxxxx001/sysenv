# sys
**English** | [简体中文](./README.md)
A cross-platform (Windows / Ubuntu) Http client & AI model lookup & PATH & environment manager in a single binary, with features split into subcommands. It bundles these capabilities:

- **PATH management**: add / remove / list / check system PATH entries, persisted automatically and applied to the current environment
- **Registry import / export**: dump PATH to a Windows Registry (`.reg`), JSON or text file; import back by merge or full replace
- **Link into PATH**: put any file into a PATH directory so it can be run from anywhere
- **Environment variables**: read / set / unset / list system variables, optionally persisted or session-only
- **AI model lookup**: query model & provider information from models.dev by name (plus an independent Chinese-model catalogue from DataLearner)
- **httpie-compatible HTTP client**: flags follow httpie conventions
- **Log viewer**: `sys log` — a colored less-style log viewer that parses Nginx / Linux / Windows / Log4j-style logs: timestamps in orange, level gradient (green for info and below, red from warning up), app markers and IP addresses in blue, HTTP methods (GET/POST, etc.) in green; `--level` filter, `--search` (n/u jump), `/` live search in the interactive view (non-follow), `--line` / `--newer` positioning, `-f` follow

```
sys (Made by Gary-china)

Usage: sys <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import / unlock
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query AI model info (DataLearner), provider info (models.dev), inspect the config (info), chat with configured providers
  net    Probe domains / URLs read from a file, or extract the valid URLs from it
  task   Query and kill processes (list PID/name/path; kill by PID or name; no args lists all)
  log    Colored less-style log viewer: parse timestamps/levels/app markers, filters, search, -f follow
  file   fd/sd-style text tool: search stdin / a file tree, in-place replace, encoding conversion, hashing, line counts
  msg    Windows scheduled reminders via msg.exe popups (--daily / --once / --cron / --list / --delete)
  data   Query / modify / convert structured data (JSON/JSONL/YAML/TOML/CSV/MD/TABLE)
```

---

## Feature overview

| Feature | Subcommand | Highlights |
| --- | --- | --- |
| PATH CRUD | `path` | `add` is current-session only by default (`--save` persists); `remove` supports `--temporary`; absolute-path normalization and case-insensitive dedup; prepend; `user`/`machine` scopes; `path unlock` (Windows) detects and releases files/directories held open by other processes |
| Registry import/export | `path export/import` | `.reg` / JSON / TXT formats; merge or `--replace` full replace; machine migration & backup |
| Link into PATH | `link` | hard link → symlink → copy fallback chain; Windows `.cmd` shim; custom command name; system or managed directory |
| Environment variables | `env` | get / set / unset / list; persisted by default; `--temporary` for the current shell only; `machine` scope |
| AI model lookup | `ai` | `ai model` (models.dev: 226 providers, 8000+ models; 24 h cache; canonical preference; no args lists all; `--date` / `--open` / `--model-type` (comma-separated values are ANDed) / `--search / --list / --json / --refresh` / `--src openrouter`); `ai cn-model` (datalearner: 1039 Chinese models; `--date` / `--model-type` filters; an exact hit also scrapes the detail page for the context length, the modalities and the **parameter scale**); `ai info` (local config lookup: `info provider` lists name/api_base/api_key/docs/console/sample (sample is an example request against that provider's own models API, e.g. `https://openrouter.ai/api/v1/models  Authorization:"Bearer sk-sample"`), `info model` lists `{provider}:{name}`, `info sale-price` scrapes the official pricing page, `info list-model` / `info models` query the provider's own models API (same as `sys ai chat --query-model`); `info providers` / `info models` accept `--free` to show the built-in free routers); `ai price` (models.dev prices, `--src openrouter` switches to the openrouter.ai catalogue, see the command reference); `ai balance` (its own subcommand: minimax / agnes / alibaba-cn / deepseek / tencent query the official balance with the api_key, everything else prints the console URL); `ai config` (export the config as codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder; `-m` multi-model selection (same format as chat); `--show` previews the target file path and content; `-f` writes the file with an automatic backup; `--update` merges in place into the default config file (backed up first); `--import` reverse-imports providers/models from a codebuddy / workbuddy / opencode / zcoder config file (backed up first; add-new wins, a local provider with the same endpoint is kept — **a provider is identified by its api_base**: entries pointing at the same endpoint merge into ONE block, never several); exports merge by api_base the same way; `--src` points at an agent config directory (non-default installs) and applies to import / export / update / list; `--list` lists only the formats that actually exist); `ai chat` (multi-provider dynamic weighted round-robin chat; `-m` overrides the model, `--list-model` / `--list-provider`; messages are truncated to the configured max_input_tokens before sending; capability fields are read-only from the local config by default (models.dev is never queried), `--update` explicitly fills the missing max_input_tokens / type and writes them back; OpenAI / Anthropic compatible); `ai server` (`sys ai server [ADDR]` serves an OpenAI-compatible API — `/v1/chat/completions` + `/v1/models`, default 127.0.0.1:10000, a bare port or IP is enough; `--help` reports the server status and prints the configured provider/model); `ai image` (OpenAI-compatible Images API `POST /v1/images/generations` for image generation: `-o` output dir / `-n` count / `-s` size / `--url` downloads the URL version; b64 is decoded and saved, the extension is detected from the file magic (png/jpg/gif/webp), multi-model weighted round-robin with the same weight reward/punishment as chat) |
| Process management | `task` | no args lists all processes; list (PID / name / path, fuzzy name match); `-o` shows the process using a port; kill by PID or name; `-f` force |
| Windows reminders | `msg` | desktop popups via schtasks + msg.exe: `--daily` / `--once` / `--cron` schedules; `--name` custom task name; `--list` / `--delete` manage; `TEXT` reminder (GBK-encoded); `--music` chime; tasks are `sysmsg-`-prefixed to isolate them from system tasks; Windows only |
| Network probing | `net` | batch-probe http(s) addresses from a file: `-i` reads the file (`#` comment lines skipped, several addresses per line, each address runs to the next whitespace with trailing punctuation trimmed); `-t domain` (default — 3 GET probes per address, average response time, printed sorted by average when all are done) / `-t tv` (body must parse as JSON, analyzed with `--sort`; old name `json`) / `-t tvs` (accepts both tv and aggregator JSON: single `.sites` / multi `.urls` groups) / `-t file` (body must be plain text) / `-t test` (probe reachability only — unreachable addresses are reported and removed from the source lines, `-o` writes the cleaned source) / `-t info` (query RDAP registration info for each `-n` domain, falling back to the parent domain when missing); tv/file modes print url / reachability / status / response time / format as each probe finishes and `-o` writes the passed urls (reachable + format matched) to a file, one per line; `--timeout` per-request timeout |
| Text search & replace | `file` | fd/sd-style: 1 arg searches stdin (an extension like `me.txt` displays that file instead; quotes force the search); 2 args search a directory tree of text/source files (`-e` / `-i` / `-t` / `-w` / `-c` / `-n` line numbers); 3 args OLD NEW PATH replace in place; `-l` counts lines (file / stdin / matching lines); `-S` size / `--newer` / `--older` time / `-d` depth filters, listing files when no PATTERN is given; `-md5 FILE [MD5]` / `-sha FILE [SHA1]` hash & compare; `--enc SRC [DST]` converts file encoding (gbk/gb2312 <-> utf-8, default target utf-8, `--from` fixes the source instead of auto-detecting, DST omitted -> stdout) |
| Data query / modify | `data` | data-compatible: selector reads `a.b[0]` / `list[*]` / `m["中文键"]` / root arrays `[0]` (`[0].PID` / `[-1]` / `[*]`; `[*].filter(pred)` filters the whole table, e.g. `ps | sys data -i table '[*].filter(Id==7360)'` — and for csv/table a **bare comparison predicate auto-filters**: `ps | sys data -i table 'Id==7360'` works directly); **Python-style list slices** `list[:2]` / `list[1:3]` / `list[1:]` / `list[-2:]` / `list[::2]` / `list[::-1]` (omitted bounds use defaults, negative indices count from the end, step may be negative, step 0 errors; read-only — plain selectors, `|count()` pipelines and projection left sides work, while `PATH = ...` writes and filter/search/remove paths reject it, e.g. `sys data 'sites[:5]|count()'` counts the first 5 sites); `PATH = VALUE` writes (`= null` deletes, `+=` / `-=` / `*=` / `/=` ops; **`PATH = @FILE`** loads a file as the value — format by extension json/jsonl/yaml/toml/csv/md, unknown extensions fall back to JSON → YAML → plain text, e.g. `cat 4k.json | sys data 'wallpaper=@file.json' --root`; **`PATH = strenv(NAME)`** reads an environment variable as a string value — an unset variable is an error, e.g. `cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root`); function expressions `list.filter(pred)` / `list.remove(pred)` / `search(pred)` / `name.endsWith("x")` / **`count()` counts matched nodes** (an array result counts its elements, a single value counts 1, no match counts 0; pipeline forms `data|count()` / `data.filter(...)|count()` / `search(...)|count()`, dot form `data.count()`, bare `count()` counts the whole document — e.g. `ps | sys data -i table '[*]|count()'` counts rows, `sys data 'sites.filter(active==true)|count()'` counts active sites) (predicate language: `$this` / `$key`, field paths, `== != > >= < <= && || !`, `contains` / `startsWith` / `endsWith` / `indexOf` / `has`; e.g. `sys data 'sites.filter(type==1)'`); **jq-style key projection pipeline** `data[*]|{id,name,pricing}` — keep only the listed keys per matched element (missing keys → null), a single match emits an object, several emit an array; the left side may be a `filter` / `search` result whose array is spread per element, e.g. `sys data 'clients.filter(name==''modelscope'')|{id,name}' -f config.yaml`; `--depth N` (default 1) removes nodes deeper than N levels; JSON / **JSONL** / YAML / TOML / CSV / MD / **TABLE** interconversion — **`-i jsonl`** (JSON Lines: one JSON value per line parses into a **root array**, so selectors / filter / `--limit` work array-style, with bare-predicate auto-filter: `sys data -f app.log 'level=="error"'`, and `.log` content-detection fallback so no `-i` is needed); **CSV and table headers are virtualised as object keys** — for `-i csv` / `-i table` the first non-blank line is the header by default (query by column name); a second line made of `----` separators is recognised as a header underline and skipped (e.g. ps' `-------` row); `--no-header` is the only way to get `col1..colN` (never auto-inferred), `--header` forces the first line as header; table folds extra cells into the last column (COMMAND with spaces) and pads missing cells; cell types inferred (number / bool / empty / string); `-f` file (extension decides the format, `-i` overrides), `-u URL` GET fetch (default json), `-w` write back, `--root` / `--compact`, `--limit N` (array root keeps only the first N rows — with `-i csv` / `-i table` this shows only the first N data rows, header still printed; the expression runs on the truncated document); non-UTF-8 input auto-decoded (UTF-8 BOM / UTF-16 BOM / GBK / GB18030, lossy fallback last — GBK device logs read directly); tab-indented YAML normalised, CJK round-trips cleanly; **a bare `{k1,k2}` projects the root document directly** (an object root keeps only those keys, e.g. `cat 4k.json | sys data '{wallpaper,warningText}'`; an array root projects every row, same as `[*]|{...}`); **the pipe is optional on a deep node**: `sites[0]{key,name}` / `sites[0].{key,name}` == `sites[0]|{key,name}`; replaces the removed `con` command — **full reference and examples in section 10** |
| Log viewer | `log` | colored less-style log viewer: parses Nginx / Linux / Windows / Log4j-style logs — **timestamps** (ISO / slash / nginx access / syslog / US formats, start-of-line or mid-line) in orange; **levels** (trace→info light-to-strong green, warning→error→fatal escalating red) as a gradient; **app markers** and **IP addresses** (IPv4 / IPv6, anywhere in a line — head or message) in blue, **HTTP methods** (request-line verbs like `GET /pages/ HTTP/1.1`, incl. no-version forms like CONNECT) in green; `--level` filters by level (warn==warning, err==error); `--search` keyword search (line numbers + reverse highlight, n/u wrap-jump in interactive mode); pressing `/` in the interactive view starts a **live search** (non-follow mode: re-highlights and jumps to the first match as you type, Esc cancels, Enter commits); `--line N` parses from line N onward (original numbers kept); `--newer TIME` shows entries at/after the given time; `-f` follows file growth with scrolling (150 ms polling, auto re-read on truncation/rotation); TTY gets a less-style interactive view (PageUp/PageDown/↑/↓/Home/End/g/G/q), pipes get plain text without ANSI |
| HTTP client | `http` | httpie-compatible flag subset; **JSON by default** (every request carries `Content-Type: application/json` unless `-f` / `--multipart` or an explicit header overrides it); JSON / form / multipart / raw body; nested JSON; download / redirect / auth / offline / curl-command export; `--help` reference; `--debug` prints the actual request & response (incl. headers) |

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
- On both platforms, `path add --save` and `env set` also **update the current process environment** (paste-ready export snippet for the current shell, registry broadcast on Windows) — persistence and immediate effect are both satisfied.
- `path add` is current-session only by default: it updates the current process and prints a paste-ready shell snippet (`export` / `$env:`) without writing anything; add `--save` to persist. Other write operations (`path remove`, `env set/unset`, `link`) persist by default; `--temporary` makes them current-session only.

---

## 1. PATH management (`path`)

### Features

- `list`: show the full effective PATH (current process + persisted scopes merged)
- `add`: append to the end; `-p/--prepend` inserts at the front; **current-session only by default** (paste-ready snippet, nothing persisted), `--save` persists for new processes; normalizes to absolute paths, deduplicates case-insensitively, skips empty entries
- `remove`: remove a directory from any scope by absolute path
- `has`: check both the current process and persisted scopes, report whether the directory is on PATH
- `export / import`: bidirectional conversion between Registry (`.reg`), JSON (`.json`) and plain text (`.txt`); `export` picks the format from the file extension: `.reg` (Windows Registry Editor format, double-click to import), `.json` (structured, scope-aware), `.txt` (one entry per line); `import` **merges** by default (dedup on absolute path, then append), `--replace` replaces the whole PATH; ideal for machine migration, backup/restore, and snapshots before batch edits
- `--scope user|machine`: choose the write scope of **`--save`**
- `--save`: persist for new processes (default `user` scope; `--scope machine` needs admin/root); without it, current session only

### Examples

```
sys path list                      # list the effective PATH
sys path add D:\tools              # current session only (print snippet, not persisted)
sys path add D:\tools -p           # prepend (still current session only)
sys path add D:\tools --save       # persist (user scope); new processes inherit it
sys path add D:\tools --save --scope machine   # machine scope (admin/root)
sys path remove D:\tools
sys path has D:\tools              # checks process + persisted scopes
sys path unlock D:\work\a.txt      # Windows: check whether a file/directory is held open by processes, list the lockers (PID/name/path) and release them
sys path export backup.reg         # Windows Registry Editor format, double-click to import
sys path export paths.json
sys path import backup.reg         # merge into user PATH (deduped)
sys path import backup.reg --replace   # full replace
sys path export paths.txt          # exchange/backup on Ubuntu
```

## 2. Link into PATH (`link`)

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

## 3. Environment variables (`env`)

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

## 4. AI model / provider lookup (`ai`)

### Command reference

| Subcommand | Purpose | Key flags / examples |
| --- | --- | --- |
| `ai model` | query the models.dev model catalogue (exact name / substring / full list) | `--search` `--list` `--limit` `--date` `--open` `--model-type` `--provider` `--price` `--json` `-o` `--src openrouter`; `sys ai model gpt-4.1` |
| `ai cn-model` | query the DataLearner Chinese model catalogue (1039 models, an independent source) | `--search` `--list` `--limit` `--date` `--model-type` `--json` `-o`; `sys ai cn-model gpt-6-1-sol` |
| `ai provider` | query the models.dev provider catalogue | `--search` `--list` `--limit` `--json` `-o`; `sys ai provider openai` |
| `ai info` | inspect the local config (provider / model / official pricing / models API) | `info provider [KW]` `info model [KW]` `info sale-price PROV` `info list-model PROV` `info models PROV`; `-o json\|csv\|yaml\|md`; `--free[=ROUTER]` (providers / models) |
| `ai balance` | query the official balance with the api_key | `sys ai balance minimax` (minimax / agnes / alibaba-cn / deepseek / tencent have real handlers, the rest print the console URL) |
| `ai price` | query model prices (models.dev by default / the openrouter catalogue) | `sys ai price openai,anthropic`; `--src openrouter` `--limit` `--json` `-o` |
| `ai chat` | chat with a configured LLM (OpenAI / Anthropic compatible) | `-m` `--list-provider` `--list-model` `--no-stream` `--debug` `--update` `--test` `--query-model` `--free` `--web[=PROVIDER]`; `sys ai chat "hi"` |
| `ai server` | serve an OpenAI-compatible API locally | `sys ai server [ADDR]` (default 127.0.0.1:10000); `--help` queries whether it is running |
| `ai image` | generate images through the Images API | `-o DIR` `-n N` `-s SIZE` `--url` `--list-provider` `--list-model`; `sys ai image "a red fox in the snow"` |
| `ai config` | export / apply / reverse-import the config for other AI tools | 8 formats; `-m` `-f` `-c` `--show` `--update` `--import` `--src` `--list`; `sys ai config codex --show` |

> Common option: `--refresh` belongs to the `ai` level and may appear anywhere after `ai` (e.g. `sys ai --refresh model gpt-4.1` or `sys ai model gpt-4.1 --refresh`); it forces a re-fetch of the 24 h cached data.

### Features

- Data source: the official public data of [models.dev](https://models.dev), `https://models.dev/api.json` (~5.3 MB, **226 providers, 8000+ models**); the `/models/` and `/providers/` pages render from this same data
- **24 h local cache**: Windows `%LOCALAPPDATA%\sys\`, Linux `$XDG_CACHE_HOME` or `~/.cache/sys/`; `--refresh` forces a re-fetch
- Matching: case-insensitive exact match on model `id` / `canonical_model_id` / `name`, with substring fallback
- **Canonical preference**: when several providers expose the same model name, the primary entry is chosen by majority vote on the `canonical_model_id` provider prefix (e.g. `gpt-4.1` → OpenAI), its detail is printed and the remaining providers are summarized; full ids (`openai/gpt-4.1-mini`) pin down one entry exactly
- `-s/--search`: substring-search list output; `--limit` controls the count (default 20)
- `--list`: paginated browsing of all models / providers
- **No args lists all**: `sys ai model` with no arguments lists every model (`--limit N` still caps the output)
- **Text lists have a header row**: the default output of no-arg / `--list` / `--search` is an aligned table headed `ID  NAME  FAMILY  CONTEXT  TYPE  LAST UPDATED` (id / name / family / context length / modality type / last updated)
- `--date YYYY-MM-DD`: show only models whose `last_updated` is **strictly after** the given date; combines with no-arg / `--list` / `--search` / name lookups (the date filter is applied before matching); a malformed date is rejected with an error
- `--open`: show only models with `open_weights: yes`; combines with `--date` and the other filters (date first, then open weights)
- `--model-type TYPE[,TYPE...]`: show only models supporting these modalities — `text / image / audio / video / pdf` (the Chinese aliases 文本 / 图像 / 语音 / 视频 are accepted too); **several values separated by a half-width `,` or a full-width `，` are ANDed** (a model must support every listed modality), e.g. `--model-type text,image` shows only models that do both text and image; combines with `--date` / `--open`
- `--src openrouter` (on `ai model`): switch the data source to the openrouter.ai catalogue — a list of `id / name / context_length / pricing.prompt / created`
- `-o/--output-format json|csv|yaml|md`: machine-readable output — `json` prints a **JSON array** (equivalent to `--json`; a single hit is still an array), `csv` prints a header + CSV table (24 columns for models, 6 for providers, RFC-4180 escaping), `yaml` prints a YAML list, `md` prints **Markdown** (`sys ai info` prints an unordered list — `- provider` with `api_base / api_key / console / docs / sample` children; `ai model` / `ai provider` / `ai cn-model` print Markdown tables with `|` and newline escaping). Works for detail, search, list and multi-match modes alike
- Model detail fields: `id / name / provider / family / description / modalities / context / output limit / cost (input/output/cache_read per 1M tokens, in USD) / reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` and more

### Examples

```
sys ai model                          # no args: list every model
sys ai model --date 2026-10-01        # only models last updated after 2026-10-01
sys ai model --date 2026-10-01 --limit 10   # above + first 10 entries
sys ai model --open --limit 10        # only models with open_weights: yes
sys ai model --open --date 2025-01-01 # combined filters: updated after 2025 and open weights
sys ai model --model-type audio --limit 10   # only models with audio support (text/image/audio/video likewise)
sys ai model --model-type text,image --limit 10  # multi-value AND: models that do both text and image
sys ai model --model-type "文本，图像" --limit 10 # Chinese aliases + a full-width comma work too
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
sys ai model --src openrouter --limit 10   # openrouter.ai source: id/name/context_length/pricing.prompt/created
sys ai model --src openrouter -o json      # openrouter source, JSON output
sys ai info provider -o md            # provider info as a Markdown table
sys ai info model -o md               # model list as a Markdown table
```

CSV columns: models `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`; providers `id,name,api,env,npm,models_count`.

#### 4.1 China AI model lookup (`ai cn-model`)

Data source: the [datalearner](https://www.datalearner.com/ai-models/pretrained-models) pretrained-model list (server-rendered HTML, about **1039 models**, fetched page by page and deduplicated by slug), fully independent from `ai model` (models.dev).

- Query & listing: exact lookup by **name** and `-s/--search` substring search (aliases match too); `--list` prints the whole catalogue; there is **no** `--open` (this source does not expose open-weights info)
- `--date YYYY-MM-DD`: filters by the **published** date shown on each card, keeping only models **strictly after** the date; used alone (without a name / search term) it lists every model published after that date
- `--model-type TYPE[,TYPE...]`: filters by **category** — `text / image / audio / video / multimodal` (the Chinese aliases 文本 / 图像 / 语音 / 视频 / 多模态 are accepted too); **several values separated by a half-width `,` or a full-width `，` are ANDed** (the category must match a keyword of every listed type, e.g. `audio` matches the 语音 category and `image` matches 视觉 / 多模态)
- **An exact hit also scrapes the detail page**: a single-model exact lookup additionally fetches that model's detail page and reports `context` (context length, e.g. `1.05M`), `modality` (input / output modalities, e.g. 文本、图像 → 文本) and `parameters` (the **model scale / parameter count**, e.g. 2350亿 / 220亿); a failure degrades silently (the field is left empty) and a "暂无数据" placeholder on the site counts as unknown and is left empty too
- Fields: `id (slug) / name / provider (publisher) / aliases / type (精选 / 预览版 / 开源模型 / 闭源模型 badges) / category (e.g. 推理大模型) / context (context length, detail page) / modality (input/output modalities, detail page) / parameters (parameter scale, detail page) / published / url`
- 24 h local cache (same cache dir as `ai model`, file `datalearner-models.json`); `--refresh` forces a re-fetch
- `--limit N` caps the list (default 20); `--json` / `-o json` print a JSON array; `-o csv` prints an 11-column CSV (`id,name,provider,aliases,type,category,context,modality,parameters,published,url`)

```
sys ai cn-model gpt-6-1-sol            # exact lookup (case-insensitive; with detail-page context / modality / parameters)
sys ai cn-model -s ernie --limit 10    # substring search (aliases match too)
sys ai cn-model --list --limit 20      # list the whole catalogue (first 20)
sys ai cn-model --date 2026-09-28      # every model published after that date
sys ai cn-model --model-type 语音       # every speech / audio LLM
sys ai cn-model --model-type 语音,多模态 # multi-value AND: both the speech and the multimodal category
sys ai cn-model -s qwen --date 2026-01-01   # search + date filter
sys ai cn-model gpt-6-1-sol --json     # JSON array output
sys ai cn-model -s ernie -o csv        # CSV output
sys ai cn-model gpt-6-1-sol --refresh  # force re-fetch (--refresh lives on `ai`)
```

#### 4.2 AI chat (`ai chat`)

Chats with the configured providers using the OpenAI `/v1/chat/completions` or the Anthropic Messages API standard.

- Config file defaults to **`~/.sysenv/config.yaml`** (a clear message is shown when missing); `-c/--config FILE` overrides it. The format is documented in `doc/config.yaml`; the sanitized template is `doc/config.example.yaml`
- `clients` holds the providers: required `name / api_base / api_key / models` (each `models` entry has `name` plus optional `weight`, default 1, **range 0-9**, and optional `max_tokens`, `max_input_tokens`, `type`, `group`); `type` is `openai` (default; `open` is accepted) or `anthropic`, and the request follows the OpenAI or the Claude API standard accordingly
  - `group`: model **group** membership; several groups are allowed, separated by half- or full-width commas (e.g. `group: fast, cheap`). The top-level `model` and `-m` accept `group:<name>` to select **every model belonging to that group** (across providers, weighted round-robin)
- **Capability fields are read-only from the local config unless `--update` is given**: `max_input_tokens` / `type` come entirely from the config file; when missing they are **never** looked up in models.dev and **nothing is written back** — chat never depends on an external data source
  - `--update`: before sending, fill the target models' **missing** `max_input_tokens` / `type` from models.dev (24 h cache) and **persist them into the config file** (best effort: lookup / write failures only warn and never block; stderr prints what was actually filled). Models whose fields are already set are not queried. Without the flag the zero-query / zero-write-back behavior above applies
- **Message length truncation**: before sending, the message character count is checked against the model's configured `max_input_tokens`; anything beyond it is cut (a truncation note goes to stderr) so the context window is never exceeded. Without a configured `max_input_tokens` nothing is truncated
- Top-level `model` selects the model:
  - missing → the first model of the first provider
  - `provider:model` → the model whose provider name and model name both match
  - `provider:*` → **all** models of that provider, picked by **weighted round-robin** on `weight` (default 1)
  - `model` (bare name) → every matching model across all providers, again weighted round-robin
  - `group:<name>` → every model across all providers whose `group` list contains the name (case-insensitive), again weighted round-robin; on miss an error lists the available groups
  - **several selections**: both the top-level `model` and `-m/--model` accept multiple choices separated by a half- or full-width comma (e.g. `agnes:*,group:cheap`), used by weighted round-robin
- `-m/--model MODEL`: **temporarily overrides** the top-level `model`; the rules are exactly the same as in the config (`provider:model` / `provider:*` / bare name / `group:<name>` / comma-separated multiple selections)
- `--list-provider`: lists every provider in the config (name / type / model count) and exits; `--list-model`: lists every model grouped by provider (with weights and groups) and exits. Both may be combined, and neither needs a message
- **Dynamic weighted round-robin**: when a request fails and that model's `weight > 1`, the weight is decremented and **written back to the config file**, then the next model is tried automatically; when a substitute succeeds and its `weight < 9`, the weight is incremented and written back. A higher weight means a higher chance of being picked, and the failure penalty / success reward keeps accumulating (weight 0 means it barely takes part in the rotation)
- The rotation state persists in `chat_state.json` next to the config, so successive calls keep rotating
- Message source: command-line arguments (joined with spaces); with no arguments, stdin is read when it is not a terminal
- Top-level `stream: true` enables SSE streaming by default (printed token by token); `--no-stream` disables it; `--debug` forces non-streaming
- `--debug`: prints the **actual HTTP request** (method / URL / headers / body) and **response** (status / headers / body) to stderr without polluting stdout
- `--free[=ROUTER]`: **ignores the config file and uses only the built-in free routers** (auto-enabled when no config file exists): `kilo-auto/free` (Kilo Gateway, anonymous) and `blockrun/free` (BlockRun anonymous free endpoint `https://blockrun.ai/api/v1`, model `nvidia/llama-3.2-11b-vision`) are always available; `openrouter/free` is appended only when `OPENROUTER_API_KEY` is set. Bare `--free` weighted round-robins across all available routers; `--free=kilo` / `--free=blockrun` / `--free=openrouter` pins one router (aliases `kilo-auto` / `kilo-auto/free` / `blockrun/free` / `openrouter/free` accepted), with a clear error when the pinned router is unavailable

- `--web[=PROVIDER]`: **search the web first, then answer** — the message is used as the query against a configured search provider (results with snippets), the results are embedded into the message as grounding context, and the model answers based on them with source links. PROVIDER: `auto` (default; prefers an entry whose name contains `bocha`, then one containing `yuanbao`, then the first entry), `bocha` (Bocha AI, `POST /v1/web-search`, get a key at open.bochaai.com) or `yuanbao` (Tencent Yuanbao Search WSA `SearchPro`, two auth modes: service API KEY via `POST https://api.wsa.cloud.tencent.com/SearchPro` (Bearer single key), or Tencent Cloud standard mode via `https://wsa.tencentcloudapi.com/` (TC3-HMAC-SHA256 signature, config `SecretId`+`SecretKey`, recommended)). The search keys come from the top-level `search:` section of the config (e.g. `- name: bochaai, key: sk-...`, `- name: yuanbao, SecretId: AKID..., SecretKey: ...`), with a clear error when missing. `--debug` prints the search request and response; `--web` conflicts with `--test`

```
sys ai chat "hi"                       # chat with the default config (~/.sysenv/config.yaml)
sys ai chat hi there                   # multiple args are joined
echo "summarize this" | sys ai chat    # pipe via stdin
sys ai chat "hi" -c doc/config.yaml    # explicit config file
sys ai chat "hi" --no-stream           # disable streaming
sys ai chat "hi" --debug               # print the actual request & response (incl. headers)
sys ai chat "hi" --free                # built-in free routers (kilo-auto / blockrun; openrouter when the key is set)
sys ai chat "hi" --free=blockrun       # pin BlockRun anonymous free endpoint (https://blockrun.ai/api/v1)
sys ai chat "weather in Shenzhen today" --web  # web search first, then answer grounded in the results (auto: bocha preferred; needs the top-level `search:` section)
sys ai chat "weather in Shenzhen today" --web=yuanbao  # pin Tencent Yuanbao Search (the entry whose name contains `yuanbao`)
sys ai chat "hi" -m agnes:agnes-3.0-flash   # override the top-level model; only that agnes model
sys ai chat "hi" -m "agnes:*,claude:claude-3-5-sonnet"  # comma-separated models, weighted round-robin
sys ai chat "hi" -m "group:cheap"      # select every model of the cheap group by group name
sys ai chat --list-provider            # list the providers in the config
sys ai chat --list-model               # list every model in the config (with weights)
sys ai server                          # serve an OpenAI-compatible API (default 127.0.0.1:10000)
sys ai server 8080                     # bare port: host stays 127.0.0.1
sys ai server 0.0.0.0                  # bare IP: port stays 10000 (exposed to the network)
sys ai server 0.0.0.0:9000             # full address
sys ai server --help                   # server status: address/port + configured providers/models
```

- `sys ai server [ADDR]`: **serves the configured providers as an OpenAI-compatible API** (default `127.0.0.1:10000`; `-c/--config` selects the config file). Exposes `POST /v1/chat/completions` (non-streaming passthrough and SSE streaming via chunked transfer) and `GET /v1/models` (every configured model, id `{provider}:{model}`). The request `model` follows the same selector rules as chat (`provider:model` / `provider:*` / bare model name / `auto` = config default). Only `type: openai` providers are served; anthropic targets get a clear error. The address may be a bare port (`8080` → host default) or a bare IP (`0.0.0.0` → port default).
- `sys ai server --help`: **queries the server status** — probes that address (GET /v1/models, 2 s timeout) to see whether a server is already running: if so it prints the current address/port, otherwise it says `server is NOT running`; in both cases it then prints the configured info grouped by provider — one provider line (name/api_base) followed by that provider's indented model lines (`{provider}:{name}`).

#### 4.3 AI image generation (`ai image`)

Generates images through a provider's OpenAI-compatible **Images API** (`POST {api_base}/images/generations`, Bearer auth) and saves them locally.

- The config file is exactly the same as for `ai chat` (default `~/.sysenv/config.yaml`, `-c/--config` overrides it); only **OpenAI-compatible providers (`type: openai`)** are supported — a `type: anthropic` target errors out clearly before the request is sent (Anthropic has no Images API)
- **Model selection follows the same rules as `ai chat`**: the top-level `model` or `-m/--model` (`{provider}:{model}` / `{model}` / `provider:*` / `group:<name>` / comma-separated multiple selections); several targets are picked by weighted round-robin on `weight`, a failure lowers the weight and moves on to the next model, a successful substitute raises it again, and the weights are written back into the config file
- **Flags**: `-o/--output DIR` output directory (default: the current directory, created automatically); `-n/--count N` how many images to generate (default 1); `-s/--size SIZE` image size (default `1024x1024`, passed to the API verbatim, e.g. `512x512` / `1792x1024`); `--url` requests the URL version and downloads it (by default `b64_json` is requested, decoded locally and saved — no second network request)
- **Saving**: the file name is `sys-ai-image-<YYYYMMDD-HHMMSS>-<index>.<ext>`, the extension is detected from the file magic (png / jpg / gif / webp), and a name collision appends `-1`, `-2`, … instead of overwriting; the saved path of every image is printed to stdout
- Prompt source: command-line arguments (joined with spaces); with no arguments stdin is read when it is not a terminal; `--debug` prints the actual HTTP request & response to stderr

```
sys ai image "a red fox in the snow"                  # default config + default model, saved into the current dir
sys ai image a red fox in the snow                    # multiple args are joined (space separated)
echo "cyberpunk city at night" | sys ai image         # stdin pipe
sys ai image "a cat under a cherry tree" -o pics -n 2 -s 512x512  # 2 images at 512x512, saved into pics/
sys ai image "poster key visual" -m bigmodel:cogview-4   # override the top-level model; only that bigmodel model
sys ai image "landscape" -m "agnes:*,bigmodel:cogview-4" # comma-separated models, weighted round-robin
sys ai image "landscape" --url -o pics                # request the URL version and download it
sys ai image --list-provider --list-model             # list the providers / models in the config
```

#### 4.4 Local config lookup (`ai info`)

Inspects the local config file (default `~/.sysenv/config.yaml`, `-c/--config` overrides) and the models.dev model prices. **With no FIELD argument it prints the provider table (name/api_base/api_key/DOCS/CONSOLE) and the full model list (`{provider}:{name}`) together.**

- `info provider [KEYWORD]` — lists every provider under `clients` with `name / api_base / api_key` plus the official `DOCS` (help docs), `CONSOLE` and `SAMPLE` URLs — `SAMPLE` is an example request against that provider's own models API, e.g. `https://openrouter.ai/api/v1/models  Authorization:"Bearer sk-sample"` (matched by provider name against a built-in table; unknown providers show `-`); with KEYWORD only providers whose name contains it are kept (case-insensitive), otherwise an error lists the available names; **defaults to a YAML list** (`-o yaml` is equivalent), `-o json` / `--json` print a JSON array and `-o csv` a CSV table (JSON/YAML fields `name` / `api_base` / `api_key` / `docs` / `console` / `sample`)
- `info model [KEYWORD]` — lists every configured model as `{provider}:{name}`; with KEYWORD only models of providers whose name contains it are kept (`provider:model` / `provider:*` select specific models); when no provider matches, models whose name contains KEYWORD are listed instead; models with `group` show their groups (text: `(group: ...)` suffix; md: `- group：...` line); `-o json` / `-o csv` / `--json` print a JSON array or CSV table (JSON fields `provider` / `name` / `groups`, CSV columns `provider,name,groups`)
- `info providers` / `info models` accept **`--free[=ROUTER]`**: the local config file is skipped and the built-in free routers are used instead — `kilo-auto/free` (anonymous) and `blockrun/free` (anonymous) are always available, `openrouter/free` is appended only when `OPENROUTER_API_KEY` is set; a bare `--free` lists every available router, while `--free=kilo` / `--free=blockrun` / `--free=openrouter` pins one (the same routers as `ai chat --free`)
- `info sale-price PROVIDER` — scrapes the provider's official pricing page and prints every model's sale price. agnes uses `wiki.agnes-ai.cn` (text / image / video models, list + current price in ¥); minimax uses `platform.minimaxi.com` (language-model input / output / cache prices, ¥ per million tokens); alibaba-cn prints the official Bailian model list & billing page; other providers print their official pricing-page URL. Pricing pages are cached 24 h, `--refresh` forces a re-fetch
- `info list-model PROVIDER` / `info models PROVIDER` — queries the provider's own models API for the models its configured api_key can actually use (the same function as `sys ai chat --query-model PROVIDER`): OpenAI-compatible providers hit `GET {api_base}/models` (the `models` path is appended to the configured api_base as-is — e.g. Baidu Qianfan configured with `https://qianfan.baidubce.com/v2` is queried at `/v2/models`, not `/v2/v1/models`; Bearer api_key), anthropic providers hit `GET {api_base}/v1/models` (`{api_base}/models` when api_base already ends with `/v1`; x-api-key / anthropic-version); `next_page` pagination is followed automatically (up to 20 pages). The text view prints an aligned `MODEL / OWNED_BY / CREATED` table (OpenAI `owned_by` / `created` and Anthropic `display_name` / `created_at` are normalized into the same columns; the `created` timestamp is shown as local `YYYY-MM-DD HH:MM:SS` in the text view); `-o json` / `-o csv` / `--json` print a JSON array or CSV table (fields `id` / `owned_by` / `created`, keeping the API's raw timestamp). Always live, no cache
- **Balance lookups** (since v0.6.17 `info balance` was promoted to its own subcommand): to query a provider's official balance with its `api_key` use **`sys ai balance PROVIDER`** (see 4.6); **model prices** (the models.dev / openrouter catalogue) are queried with **`sys ai price`** (see 4.5)

```
sys ai info                              # no field: provider table + full model list
sys ai info provider                     # default YAML: every configured provider
sys ai info provider -o yaml             # same as the default (YAML list)
sys ai info provider agnes               # only providers whose name contains agnes (YAML)
sys ai info provider -o json             # every provider as a JSON array
sys ai info provider agnes -o csv        # filtered + CSV output
sys ai info provider --json              # same as -o json
sys ai info model                        # every model as provider:model
sys ai info model modelscope             # only modelscope's models
sys ai info model deepseek               # falls back to matching model names
sys ai info model agnes:3.0              # one model: agnes models whose name contains 3.0
sys ai info model modelscope:*           # every modelscope model
sys ai price openai,anthropic            # model prices of two providers (models.dev)
sys ai price "openai，deepseek"          # a full-width comma works too
sys ai price --src openrouter            # prompt/completion prices of every openrouter.ai model
sys ai price deepseek --src openrouter   # openrouter source + model-name filter
sys ai price --src openrouter --limit 10 # only the first 10 entries
sys ai balance minimax                   # query MiniMax's official balance with its api_key
sys ai balance alibaba-cn                # Alibaba Bailian: official limits + console hint
sys ai info providers --free             # list providers with the built-in free routers (no config file)
sys ai info models --free=blockrun       # list models pinned to the blockrun free router
sys ai info sale-price agnes             # scrape agnes' official pricing page (all models)
sys ai info list-model agnes             # live model list from agnes' own models API
sys ai info list-model anthropic -o json # anthropic: /v1/models, JSON array output
sys ai info models baidu                 # same as --query-model: baidu's /v2/models (live)
```

#### 4.5 Model price lookup (`ai price`)

Model prices come from [models.dev](https://models.dev) (the default, the same source as `ai model`) or from the openrouter.ai official pricing catalogue (`--src openrouter`).

- **`src=models` (default)**: `sys ai price openai,anthropic` resolves the comma-separated provider names in models.dev and lists **every model** of those providers with its prices (input / output / cache_read per 1M tokens, converted to USD; `--refresh` forces a re-fetch of the 24 h cache). Both a half-width `,` and a full-width `，` work
- **`src=openrouter`**: `sys ai price --src openrouter` fetches the prompt / completion prices of the whole openrouter.ai catalogue (per 1M tokens, 24 h cache, `--refresh` re-fetches); no provider name is needed, comma-separated model ids / name keywords filter the list (`sys ai price deepseek --src openrouter`), `--limit N` caps it and `--json` / `-o json|csv|yaml|md` switch the output format
- Complements `ai info` (the local config lookup): `info` shows what the config holds, `price` shows the official pricing of each model

```
sys ai price openai,anthropic            # models.dev: prices of every model of two providers
sys ai price "openai，deepseek"          # a full-width comma works too
sys ai price --src openrouter            # openrouter.ai: prices of every model (per 1M tokens)
sys ai price deepseek --src openrouter   # openrouter catalogue + model-name filter
sys ai price --src openrouter --limit 10 # only the first 10 entries
sys ai price --src openrouter -o csv     # CSV output
```

#### 4.6 Official balance lookup (`ai balance`)

Queries a provider's official balance in real time with its configured `api_key` (**always live, never cached**). `-c/--config FILE` selects the config file; `PROVIDER` is a provider name under `clients` (case-insensitive, unique substring matching).

- **`minimax`**: the official `GET /v1/token_plan/remains` — lists the remaining / total / used quota of every model under the Token Plan; without a Token Plan subscription (pay-as-you-go) it prints the platform balance page
- **`agnes`**: an OpenAI-compatible billing probe (credit_grants / subscription / usage; with a console hint when no balance field is returned)
- **`alibaba-cn`** (aliases alibaba / dashscope / bailian): the official `GET /api/v1/models/limits` — the usage limits / rate-limit quotas of every model under that key (it also verifies the key; there is no key-level cash-balance endpoint, so it points to the Bailian console)
- **`deepseek`**: the official `user/balance` real query
- **`tencent`** (aliases tencentmaas / tokenhub): the TokenHub control plane needs a SecretId / SecretKey (different from the configured API key), so the console instructions are printed
- **`modelscope` / `sensenova` / `bigmodel` / `amd` / `anspire`**: no key-level balance endpoint is public, so the command prints the matching console URL
- An unknown provider name is an explicit error

```
sys ai balance minimax              # MiniMax Token Plan remainders (or the pay-as-you-go entry point)
sys ai balance deepseek             # DeepSeek user/balance real query
sys ai balance alibaba-cn           # Alibaba Bailian: official limits + console hint
sys ai balance tencent              # TokenHub console instructions (SecretId/SecretKey)
sys ai balance agnes -c test\config.yaml   # a specific config file
```

#### 4.7 Config export / apply (`ai config`)

Exports the local config's `clients` section into another tool's format, or applies it straight to that software's config file:

* `ai config FORMAT [-m MODEL] [-f FILE] [-c FILE] [--show] [--update]`; `FORMAT`:
  * **`codex`** (TOML): one `[model_providers.<name>]` table per provider (`name` / `base_url` / `env_key` / `wire_api = "chat"`). codex reads keys from env vars only, so the export names them `SYS_<PROVIDER>_API_KEY` and prints `export` hints to stderr; the top-level `model` selects the first model as `provider.model`
  * **`opencode`** (JSON): one provider entry per configured provider (`npm: "@ai-sdk/openai-compatible"` + `options.baseURL` / `options.apiKey` inline + `models` map) — mergeable into `opencode.json`
  * **`litellm`** (YAML): one `model_list` entry per model; the exposed name is `{provider}:{model}` (collision-free) routing to `openai/{model}`, with `api_base` / `api_key` inline
  * **`freellmapi`** (JSON): a `customProviders` array, one `baseUrl` / `label` / `models` entry per provider (`supportsTools: true`) — mergeable into `freellmapi.config.json`
  * **`codebuddy`** (models.json): one entry per model with `id` / `name` / `vendor` / `url` (`{api_base}/chat/completions`) / inline `apiKey`; `maxInputTokens` / `maxOutputTokens` / `supportsImages` are added when the config has `max_input_tokens` / `max_tokens` / a `type` containing `image`; duplicate model ids are deduped and `availableModels` stays in sync; default path `~/.codebuddy/models.json`
  * **`workbuddy`** (models.json, **plain top-level array**): entries carry the same fields as codebuddy (`id` / `name` / `vendor` / `url` / `apiKey` / optional `maxInputTokens` / `maxOutputTokens` / `supportsImages`); the real workbuddy schema is a top-level array (not a `{"models":...}` wrapper), so the export is an array and `--update` keeps it an array; default path `~/.workbuddy/models.json`
  * **`trae`** (trae_cli.yaml): a YAML `models` list; OpenAI-compatible providers use the `open_ai` block, anthropic the `claude` block, deduped by model id; default path Windows `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml` (Linux `$XDG_CONFIG_HOME` or `~/.config/trae_cli/trae_cli.yaml`)
  * **`zcoder`** (ZCode `config.json`): one `provider.<id>` block per provider (id = provider name lowercased, invalid chars → `-`) with `kind` (anthropic / openai-compatible), `options.baseURL` / `options.apiKey`, and `models` (an object keyed by model id); `model.main` / `model.lite` point at the selected model (`provider/model`); `--update` keeps existing provider blocks and other top-level keys and fills `main/lite` only when absent; default path `~/.zcode/cli/config.json` (Windows `%USERPROFILE%\.zcode\cli\config.json`)
* `-m/--models MODEL` (omitted = **the same default models `sys ai chat` uses**: the top-level `model:` selector, or the first provider's first model when none is configured): the same format as `sys ai chat -m` — `provider:*` (a provider's whole model list) / `provider:model` (one model) / bare model name (matched across providers) / `group:<name>` (every model of that group), half-/full-width comma separated for multiple selectors, overlapping selections deduped (no match → clear error); the legacy positional `SELECT` is equivalent to a single `-m` value (mutually exclusive)
* `--show`: **inspect only, no write** — prints the target software's config file path first (codex → `~/.codex/config.toml`, opencode → `~/.config/opencode/opencode.json`, litellm → `~/.litellm/config.yaml`, freellmapi → `freellmapi.config.json` in the current directory, codebuddy → `~/.codebuddy/models.json`, workbuddy → `~/.workbuddy/models.json`, trae → `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`, zcoder → `~/.zcode/cli/config.json`; a missing default path prompts `-f FILE`), then prints the config that would be applied to that software
* `-f/--file FILE` **writes the file** (stdout by default): the existing file is first backed up to `FILE.bak-YYYYMMDD-HHMMSS` (same directory), then the actual change is reported after writing (exported model count / target file path / backup file path); a missing target file is created and reported; `-c/--config` selects the config file
* `--update` **in-place merge** (codebuddy / workbuddy / trae / zcoder): looks up the software's default config file for the format (`-f` overrides the path); a missing file prints `config file not found ... nothing to update`; an existing file is backed up to `.bak-*` first, then the missing selected models are merged in (existing entries and other top-level keys are kept), and success reports `added N model(s): ...`; mutually exclusive with `--show`
* `--import` **reverse import** (codebuddy / workbuddy / opencode / zcoder only; mutually exclusive with `--show` / `--update` / `--list`): reads provider/model params from the software's config file and merges them back into the local config `~/.sysenv/config.yaml` (`-c` picks another target; `-f FILE` names the source file, defaulting to the format's default path; a missing source errors out). For `codebuddy` / `workbuddy` a `models.json` is parsed (array root or `{"models":[...]}` root): entry `id` → model name, `url` minus the trailing `/chat/completions` → `api_base`, `apiKey` → `api_key`, `maxInputTokens` / `maxOutputTokens` / `supportsImages` map to `max_input_tokens` / `max_tokens` / `type`; a meaningful `vendor` becomes the provider name, otherwise (workbuddy writes a useless `Custom` for hand-added models) the `api_base` host is used. For `opencode` / `zcoder` each `provider.<id>` block's `options.baseURL` / `options.apiKey` / `models` are parsed (zcoder's `kind: anthropic` → `type: anthropic`). Merge policy: **add-new-first, keep existing** — a provider whose api_base is missing locally gets a full block appended (backed up first), an existing provider only receives locally missing model entries, matching models are kept locally and reported as `skipped`; entries missing `id` / `url` / `apiKey` (or an opencode / zcoder block without a key / without models) are skipped and reported. **A provider's identity is its api_base**: entries pointing at the same endpoint (trailing slashes and case ignored) coming from the source are always merged into ONE block (the first entry's name / kind / api_key wins; models are merged and deduped by name), so config.yaml never receives the same endpoint as several entries; entries at a different api_base are distinct providers even when their names match. Loading a config that contains duplicate api_bases prints a warning (non-blocking). **Exports merge the same way**: all 8 formats (codex / opencode / litellm / freellmapi / codebuddy / workbuddy / trae / zcoder) emit ONE entry/block per api_base (trailing slashes and case ignored; the first block's name/key wins, models are merged across same-endpoint blocks and deduped by name), never creating several entries for the same endpoint; same-name providers at different endpoints stay distinct, and a name collision on the entry key is resolved as `name@host` (e.g. `openroute@openrouter.ai`). When everything is already present it prints `nothing to import` and writes nothing
* `--src DIR` **agent config directory**: the format's conventional config file name (codex `config.toml`, opencode `opencode.json`, litellm `config.yaml`, freellmapi `freellmapi.config.json`, codebuddy / workbuddy `models.json`, trae `trae_cli.yaml`, zcoder `config.json`) is looked up directly inside `DIR`, replacing the default home-relative path — for agents installed at a non-default place, across `--show` / `--update` / `--import` / `--list` (`-f FILE` always wins over `--src`)
* `--list`: prints only the formats whose config file **actually exists** (at the default location, or under `--src DIR`) and their paths; missing formats are filtered out; when none exist it prints `no existing config files found`; FORMAT may be omitted with `--list`; mutually exclusive with `--show` / `--update`)

```
sys ai config codex --show               # print codex config path + config to apply (no write)
sys ai config codex -f ~/.codex/config.toml   # back up old config → write → report the change
sys ai config codex                      # all models as codex TOML (stdout)
sys ai config opencode -f opencode.json  # write to a file (previous file backed up)
sys ai config litellm -m "agnes:*"       # only agnes' models (-m, same as chat)
sys ai config codebuddy -m "modelscope:*,minimax:MiniMax-M2.7"  # multiple selectors
sys ai config trae --update              # back up default trae_cli.yaml → merge missing models → report
sys ai config workbuddy --update -f ~/.workbuddy/models.json   # update a specific file
sys ai config zcoder -m "amd:*" --update   # merge into ZCode config.json (existing providers and other keys kept)
sys ai config workbuddy --import         # import ~/.workbuddy/models.json back into ~/.sysenv/config.yaml (backup first)
sys ai config opencode --import --src D:\tools\opencode   # import opencode.json from a specific agent dir
sys ai config zcoder --import -f some\config.json   # import from a specific source file
sys ai config codebuddy --import -c test\config.yaml   # import into a specific sysenv config
sys ai config --list --src D:\tools\opencode        # list only the configs existing under that dir
sys ai config --list                    # list only the configs existing at the default locations
```

#### 4.8 Text search & replacement (`file`)

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
* `-n` — prepend the line number to every printed line (stdin search; directory search output is already `path:line:content`)
* `-l` — count lines instead of printing them: `-l FILE` counts the file's total lines, no argument counts stdin; with a PATTERN it counts the **matching** lines (stdin prints `N`; a tree prints `path: N` per file, files with no match are omitted)
* `-md5 FILE [MD5]` / `-sha FILE [SHA1]` — **hash mode** (highest priority). Hashes FILE with MD5 / SHA-1 and prints the lowercase hex digest; a second argument compares against the expected value — equal prints `match: <digest>`, unequal prints `mismatch: computed <digest> != expected <expected>`; the expected value is case-insensitive, but an invalid length/character set errors with `expected md5/sha1 must be N hex characters`. Large files are hashed streaming (64 KiB chunks, bounded memory); giving both `-md5` and `-sha` is an error; other single-dash args (e.g. `-m`) are untouched (the markers must match `-md5`/`-sha`/`--md5`/`--sha` exactly)
* `--enc SRC [DST]` — **encoding-convert mode** (priority above hash). Converts a text file between `utf-8` / `gbk` / `gb2312` (**default target utf-8**; `--from ENC` fixes the source encoding, otherwise auto-detected; `--to ENC` sets the target). With DST the output is written to that file, without it stdout; **DST must differ from SRC** (no in-place overwrite). `gb2312` uses the GBK codec (GB2312 is a subset of GBK); auto-detection follows the same heuristic as `data` (UTF-8 strict, cross-checked against GBK), and an undetectable file errors with a `--from` hint; a UTF-8 BOM is stripped and never re-emitted. Aliases `--encode` / `--convert`; `-f ENC` equals `--from ENC` (use the double-dash `--enc` — a single-dash `-enc` would be eaten by the registered `-e` short flag)

File attribute filters (fd-style, combine with search / replace / listing):

* `-S SIZE` / `--size SIZE` — only files at least SIZE bytes. A plain number is bytes; `2k`/`2m`/`2g`/`2t` (or kb/mb/gb/tb) scale by 1024, decimals allowed (`1.5m`), case-insensitive
* `--newer TIME` — only files modified at/after TIME; `--older TIME` — only files modified before TIME. Format `YYYY-MM-DD [HH:MM[:SS]]` (`T` or slash separators work too), interpreted in the local timezone
* `-d NUM` — descend at most NUM levels of subdirectories (`-d 0` = current dir only)

Giving any filter without a PATTERN enters **list mode** (one path per line, current dir by default; `-e`/`-t` still apply); with a PATTERN or OLD NEW the filters combine with string search / in-place replacement.

Traversal: hidden entries (`.` prefix) and common noise directories (`.git` / `node_modules` / `target` / `dist` / `build` / `__pycache__` ...) are skipped; binary files (NUL bytes) are skipped; replacement honors `-e` / `-t` / `-i` / `-w` too.

```
sys file hello                              # search stdin (type a.txt | sys file hello)
sys file -n hello                           # search stdin, print line numbers
sys file me.txt                             # display the file me.txt
sys file "me.txt"                           # quoted: still a string search (find me.txt in stdin)
sys file hello D:\projects                  # search the D:\projects tree
sys file hello D:\projects -e py -e md      # only .py and .md
sys file HELLO D:\projects -i               # case-insensitive
sys file hello D:\projects -t               # plain text only (no source code)
sys file hello D:\projects -w -c 2          # whole words + 2 lines of context
sys file -l me.txt                          # count the total lines of me.txt
type a.txt | sys file -l                    # count the total lines of stdin
type a.txt | sys file -l hello              # count the stdin lines matching hello
sys file -l hello D:\projects               # per-file matching line counts (path: N)
sys file hello hi D:\projects               # replace hello -> hi in place
sys file HELLO hi D:\projects -i            # case-insensitive replacement
sys file -S 2m                              # list files >= 2 MiB in the current dir
sys file -S 5000 D:\data                    # list files >= 5000 bytes under D:\data
sys file -S 2m -e bin -d 1 D:\data          # >= 2MiB .bin files, 1 level deep
sys file --newer "2026-10-01" D:\data       # files modified at/after 2026-10-01
sys file --older "2026-10-01 12:00" D:\data # files modified before that time
sys file hello D:\data -S 1m                # search hello only in files >= 1 MiB
sys file -md5 me.txt                        # print the MD5 digest of me.txt
sys file -md5 me.txt 900150983cd24fb0d6963f7d28e17f72   # compare: match / mismatch
sys file -sha me.txt da39a3ee5e6b4b0d3255bfef95601890afd80709  # SHA-1 likewise
sys file --enc old.log new.log --from gbk   # convert a GBK log to UTF-8 into new.log
sys file --enc old.log                      # auto-detect, print UTF-8 to stdout
sys file --enc a.txt b.txt --to gbk         # UTF-8 -> GBK
```

## 5. Process management (`task`)

### Features

- **No args lists all**: `sys task` with no subcommand lists every process, equivalent to `task list`
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

## 6. Windows scheduled reminders (`msg`)

Desktop pop-up reminders via Windows Task Scheduler (`schtasks`) + `msg.exe`. All self-created tasks use the `sysmsg-` prefix, isolated from system tasks — `--list` / `--delete` only touch `sysmsg-` tasks and never affect others.

### Features

* **Windows only** (depends on `schtasks` and `msg.exe`)
* `--daily HH:MM[:SS]`: daily reminder (seconds are honored by the scheduler; both `20:00:02` and `20:00` work)
* `--once HH:MM[:SS]`: one-time reminder (if today's time has already passed it rolls to tomorrow); after firing it **auto-deletes itself** (together with its `.cmd` / `.txt` files)
* `--cron "MIN HOUR DOM MON DOW"`: a crontab five-field expression (`*` / `*/n` / single value / comma list), mapped onto the subset `schtasks` supports:
  * `m h * * *` → daily at h:m; `m * * * *` → minute m of every hour; `m */n * * *` → minute m every n hours; `*/n * * * *` → every n minutes; `* * * * *` → every minute
  * `0 20 * * 1` → every Monday 20:00 (both DOW 0 and 7 mean Sunday); `0 20 1 * *` → the 1st of every month at 20:00
  * Constraints: MINUTE and HOUR must both be concrete values to use WEEKLY / MONTHLY; the MONTH field is not supported (Task Scheduler cannot schedule by month); specifying DOM and DOW together (OR semantics) cannot be expressed in a single task and must be split; `*/n` / list / range combinations are not supported (a hint is shown)
* `--name NAME`: custom task name (auto-generated from the time by default, e.g. `daily-200002` / `once-200002` / `cron-...`); the name **may only contain letters, digits, `-` and `_`** (guards against `schtasks /TN` injection)
* `--list`: list all `sysmsg-` tasks with next-run time / status / reminder text
* `--delete NAME`: delete one `sysmsg-` task (the `sysmsg-` prefix is optional; tasks without the prefix are untouched), and clean up its `.cmd` / `.txt` files
* `TEXT` (positional, required in add mode): the reminder text; written to `.txt` as **GBK** (`msg.exe` decodes with the system ANSI codepage 936, so raw UTF-8 bytes garble); the popup stays until acknowledged
* `--music[=FILE]`: chime on reminder — `--music` plays the Windows startup sound by default (candidate files under `C:\Windows\Media`; errors if missing); `--music=FILE` specifies a tune file (wav / mp3 etc., using the `=` form to avoid clashing with `TEXT`), played asynchronously via WMPlayer.OCX
* Files live in `%LOCALAPPDATA%\sysmsg` (the `.cmd` launcher and `.txt` content)

### Examples

```
sys msg --daily 20:00:02 "Time to rest"        # daily popup at 20:00:02 (seconds optional)
sys msg --once 09:30 "Meeting" --name meeting  # one-time (rolls to tomorrow if passed), self-deletes after firing
sys msg --cron "0 20 * * *" "Off work"         # daily at 20:00 (cron subset)
sys msg --cron "*/5 * * * *" "Stand up"        # every 5 minutes
sys msg --cron "0 20 * * 1" "Monday meeting"   # every Monday 20:00 (DOW 0/7 = Sunday)
sys msg --daily 20:00 "Drink water" --music    # default Windows startup sound
sys msg --daily 20:00 "Relax" --music=C:\a.wav # specific tune (wav/mp3, `=` form avoids clashing with text)
sys msg --list                                 # list all sysmsg- tasks (next run / status / text)
sys msg --delete meeting                       # delete (prefix optional), cleans up .cmd/.txt
```

***

## 7. httpie-compatible HTTP client (`http`)

Flags follow [httpie](https://httpie.io) (subset).

### Features

- Method auto-detection: GET when the URL carries no method; automatic POST when data items are present
- JSON by default: `key=value` items build a JSON object; `-f/--form` switches to forms, `--multipart` for file uploads
- Full request-item syntax (httpie-compatible): data fields, raw JSON values, query parameters, request headers, multipart files, `@file` raw bodies, nested JSON construction (`a[b][c]=v`, `a[]=v`, `a[1]=v`)
- Output control: `-p/--print` (any combination of `BHbh`), `-h` headers only, `-b` body only, `-m` status line, `-v` everything; terminal default `hb`, pipes/redirection default to body only
- `-o/--output` saves the body to a file (the rest goes to stderr), `-d/--download` wget-style download
- Auth: `-a user:pass` Basic, `-A bearer -a TOKEN` Bearer token
- Network behavior: `-F/--follow` redirects, `--max-redirects`, `--timeout`, `--proxy`, `--verify no` skips TLS verification, `--offline` builds & prints the request without sending, `--curl` builds & prints the equivalent curl command without sending (implies `--offline`), `-I/--ignore-stdin`
- `--check-status`: exit code 3 for 3xx, 4 for 4xx, 5 for 5xx (script-friendly)
- `--help`: prints the flag reference with examples (`-h` inside `http` keeps the httpie meaning of "response headers only")
- `--debug`: prints the **actual HTTP request** (method / URL / headers / body, incl. Content-Length) and **response** (status line / headers / body) to stderr while stdout keeps its normal output — handy for verifying what is really sent over the wire
- `--curl`: prints a fully **equivalent curl command** (method / URL / query / headers / body / auth / proxy / redirects / timeout / TLS / output) instead of sending; JSON, form and multipart (`-F` text fields and file uploads) are converted from what is actually sent; `--file FILE` stays `--data-binary @FILE`, other raw bodies are inlined; values are POSIX single-quoted (runs directly in bash / zsh / WSL / git-bash / PowerShell)
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
sys http --curl pie.dev/post name=John age:=29   # print the equivalent curl command (no send)
sys http --help                            # print the flag reference with examples
sys http --debug pie.dev/post a=1 b:=2     # print the actual request & response (incl. headers)
sys http POST pie.dev/post --raw '{"a":1}' # explicit raw body
sys http pie.dev/post -- -name=foo         # field names starting with - go after --
echo '{"a":1}' | sys http POST pie.dev/post  # stdin as raw body
sys http --verify no https://self-signed.example  # skip certificate verification
```

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
`--check-status` `--offline` `--curl` `--verify` `-I/--ignore-stdin` `--default-scheme`

`--debug` `--help`

> Note: inside the `http` subcommand, `-h` means httpie-style "print response headers only", so use `sys http --help` for help.

---

## 8. Network probing (`net`)

Reads http(s) addresses from a file and probes each one for reachability and response format.

### Features

- `-i FILE`: the input file is read line by line; lines starting with `#` (after leading whitespace) are skipped; a line may hold **several addresses** — an address starts at `http` (case-insensitive), runs to the next whitespace, and has trailing punctuation trimmed (`http://a.com/x,` → `http://a.com/x`); the scheme is normalized to lowercase and the host is validated
- **duplicate detection (all modes)**: addresses already checked in this run are cached; a repeated address (host case and default ports are normalized) is **not probed again and removed directly** — in test mode it follows the deletion pipeline (single-address line deleted / multi-address line drops only that address), tv/file modes do not write it to `-o`, domain mode skips it in probing and the sorted report; printed as `[DUP] <url> duplicate (already checked), skipped`, the summary appends `, N duplicate` when any exist
- `-t domain` (default): **3 consecutive GET** requests per address, averaged; once all addresses are probed they are printed sorted by average response time **ascending**, unreachable ones last with a reason
- `-t tv` (old name `json`): GET each address; the body must be a **valid tv JSON document** to pass — it must parse as JSON (lenient: a leading BOM is stripped first, then a strict parse, then a retry with trailing commas stripped) and carry a non-empty `sites` array whose every element has a `name` key (`lives`, `parses` and `rules` are optional); the data is then analyzed and the addresses are sorted with `--sort`:
  - `--sort count`: total number of `.sites` nodes, **desc**
  - `--sort name`: number of `.sites.name` entries containing the keyword (default `4k`, case-insensitive; customize with `--words KEYWORD`, e.g. `--words hd`), **desc**
  - `--sort speed`: downloads the `.spider` url (string `url;md5;...` keeps the part before `;`, an array uses its first string element) and sorts by download speed **asc**
  - `--sort` is repeatable (e.g. `--sort count name`), applied in order for multi-key sorting; **defaults to `count, name` when omitted**
  - each passed address prints its url, followed by a `# count=<N> 4k=<N> speed=<speed or failure reason>` test-info line (the `4k` label is the `--words` keyword itself, e.g. `hd=<N>` with `--words hd`); **screen and `-o` share the same format** (url + `#` line); unreachable / non-JSON addresses are still reported as `[NO]` / `[FMT]` and not written to `-o`
- `-t file`: GET each address; the body must be **plain text** (valid UTF-8, non-binary, and not JSON) to pass
- `-t url`: **extracts offline** every valid http(s) URL from the file — no network probing at all; the same input convention applies (`#` comment / empty lines skipped, several addresses per line, trailing punctuation trimmed, scheme normalized to lowercase and the host validated); duplicate addresses (host case / default ports normalized) are skipped and reported as `[DUP]`, each unique URL is printed once per line in first-seen order; `-o FILE` writes the extracted unique URL list (one per line); a file with no URLs is not an error (summary `0`, `-o` gets an empty file)
- `-t tvs`: inherits every `-t tv` feature and additionally accepts **aggregator JSON** (e.g. `{"urls":[{"url":"...","name":"..."}, ...]}`). Each body may be one of two formats:
  - **single warehouse**: tv JSON (`sites` non-empty, every element has `name`) — analyzed with the tv `--sort` rules (count / name / speed, default `count name`), info line `# count= name= speed=`
  - **multi warehouse**: a `.urls` array only — only the node count is computed (`# count=N`), sorted by count **desc**
  - any other JSON fails with `[FMT]` and is not output
  - the final output is **grouped**: single warehouse (header `--- 单仓 (N) ---`) first, multi warehouse (`--- 多仓 (N) ---`) second, identical on screen and in `-o`
- `-t test`: probes **reachability only** (any HTTP response counts as reachable, same rule as tv/file):
  - every line is reported as soon as it is processed, with a `[line/total]` progress counter (e.g. `[3/6]`, total counts empty and comment lines): each address shows `[OK] <url> time=...` or `[NO] <url> error=<reason>`, and the line ends with `-> kept` / `-> line deleted` / `-> kept (bad address removed)`; empty lines, `#` comment lines and lines without any address show `[skip]`
  - unreachable addresses are removed from the source: a single-address line that is unreachable is deleted; a multi-address line keeps its reachable addresses and drops only the unreachable ones; a line is deleted entirely only when every address on it is unreachable
  - empty lines, `#` comment lines and lines without any address are always kept
  - `-o FILE` writes the **cleaned source result** (kept lines verbatim, in original order), one line each
  - `-x SUFFIX` (repeatable, e.g. `-x m3u -x zip`): addresses whose file-name suffix matches (case-insensitive, leading dot optional) are **removed directly without probing** — e.g. with `-x m3u`, `https://www.main.com/123.m3u` is removed without a request; a single-address line is deleted, a multi-address line drops only that address, printed as `[X] <url> excluded (-x m3u)`. Only active in `-t test` mode; other modes ignore it with a warning
- tv/tvs modes probe addresses **in parallel**: `--thread NUM` sets how many addresses are tested at once (default: **number of CPU cores**); each address prints one `[OK]` / `[FMT]` / `[NO]` line as soon as it finishes (status + response time), then the sorted/grouped report and the summary line follow when all are done
- tv/tvs modes support **inline remarks**: when a `#` follows the address (after whitespace) in an input line, the text after `#` is reported as the address's **note** — appended as `note=...` to the per-address result line, and shown as `note=... count=...` in the `#` test-info line of the final sorted/grouped output and the `-o` file (the remark of the line's last address is used; e.g. `https://a/x.json  #二月红接口`)
- tv/file modes print one result **as soon as each probe finishes**: url / reachability / status code / response time / actual format (json / text / binary)
- `-o FILE`: writes every **passed** url (reachable and format-matched) to the file (tv/file modes only)
- `-t info`: queries the **registration info** of each `-n DOMAIN` via RDAP (bootstrapped through `rdap.org`) — registrar / registrant entity (registrant + org) / email / address / **country** / created / expires / status / nameservers; when the registry response hides the registrant, the **`related` RDAP link is followed automatically** (ICANN profile: the registrar-level RDAP server carries the full entity, e.g. 163.com → MarkMonitor → 广州网易计算机系统有限公司); when a name has no registration data, the **parent domain is tried next** (`a.b.example.com` → `b.example.com` → `example.com`), annotated as `no registration data, using parent ...`; `-n` is repeatable (`-n a.com -n b.com`); this mode needs no `-i` (`-o` / `-x` / `--sort` are ignored with a warning)
- `--timeout SECONDS`: per-request timeout (default 10 seconds)

### Examples

```
# sample urls.txt input
# comment lines are skipped
https://api.example.com/v1/data   # first address
https://www.example.com/readme.txt
see https://a.com/x, and https://b.com/y   # several addresses per line
```

```
sys net -i urls.txt -t domain              # 3 probes averaged, sorted by average response time
sys net -i urls.txt -t tv -o ok.json.txt    # JSON analyzed + sorted (--sort count name), passed urls with # info lines written
sys net -i urls.txt -t tvs -o ok.groups.txt # accepts tv + aggregator JSON: single (.sites) / multi (.urls) groups, multi sorted by node count desc
sys net -i urls.txt -t file -o ok.txt      # write reachable plain-text urls to a file
sys net -i any.txt -t url -o urls.txt      # extract all valid http(s) URLs from the file offline (deduplicated), no probing
sys net -i urls.txt -t test -o clean.txt   # probe reachability, remove unreachable lines, write the cleaned source to clean.txt
sys net -i urls.txt -t tv --timeout 5      # 5 s per-request timeout
sys net -i urls.txt -t tv --thread 8        # tv/tvs parallel probing with 8 concurrent addresses (default: CPU cores)
sys net -t info -n www.example.com         # query registration info; falls back to the parent domain when missing
sys net -t info -n a.com -n b.com          # query several domains at once
```

---

## 9. Log viewer (`log`)

A colored less-style log viewer: `sys log --file app.log [--level LEVEL] [--search KW] [--line N] [--newer TIME] [-f]`. It parses each entry's **timestamp** (many common formats), **log level**, **app marker**, **IP addresses** (IPv4 / IPv6) and **HTTP methods** (GET/POST, etc.) and colors them semantically; on a TTY it enters a full-screen interactive view (like less), where **`/` starts a live search when not in `-f` mode** (typing re-highlights and jumps to the first match, Esc cancels, Enter commits, n/u keep jumping), while pipes / redirects get plain text (no ANSI).

### Features

* **Timestamp parsing (rendered orange RGB(255,165,0))** — the following formats are all recognized as the start of a log entry:
  * ISO: `2024-01-15 10:23:45`, `2024-01-15T10:23:45.123Z` (milliseconds optional); Log4j comma-milliseconds: `2024-01-15 10:23:45,123`
  * Slash: `2024/01/15 10:23:46`
  * **nginx access log**: `15/Jan/2024:10:23:49 +0800` — a timestamp **mid-line** is recognized too (first 80 bytes scanned), and the prefix (e.g. `127.0.0.1 - - [`) is kept intact
  * syslog: `Jan 15 10:23:48` (no year → current year; falls back to the previous year when the time is >1 day in the future); US: `01/15/2024`
  * Timestamps with a timezone are interpreted wall-clock style (`10:23:45 +0800` displays as `10:23:45+08:00`, no UTC conversion)

* **Level gradient**: `trace` light green(178,230,178) → `debug` green(102,204,102) → `info` green(0,180,0) — **green for info and below, lighter = lower level**; `warning` red(255,90,90) → `error` dark red(204,0,0) → `fatal` deepest red(140,0,0) bold. Aliases: `warn`==`warning`, `err`==`error`, `critical/crit/alert/emerg`==`fatal`

* **App markers (dark yellow RGB(184,134,11))**: tokens right after the timestamp such as `[http-nio-8080-exec-1]`, `app[1234]`, `[01-node-5]`

* **`--file FILE`**: log file to parse (default: stdin; `-` also means stdin)

* **`--level LEVEL`**: exact level filter (`trace | debug | info | warn | warning | error | fatal`); lines without a level are dropped

* **`--search KEYWORD`**: case-insensitive keyword search — line numbers shown (dim gray RGB(130,130,130)), matches reverse-highlighted; **n jumps to the next match / u to the previous** in interactive mode (wraps around, match centered)

* **`--line N`**: parse only from line N onward (original line numbers kept)

* **`--newer TIME`**: show only entries at/after the given time (continuation lines without timestamps are kept)

* **`-f`**: follow the file with scrolling display — 150 ms polling incremental reads, auto re-read on truncation / rotation; files only (stdin piped with `-f` errors, exit code 1)

* **Interactive keys** (TTY): `PageUp` / `PageDown` page, `↑` / `↓` line, `Home` / `End` / `g` / `G` jump, `q` / `Esc` / `Ctrl-C` quit, `n` / `u` search jump; status bar: `  filename  Lfirst-last/total  level=..  "kw" i/N  following...  q=quit PgUp/PgDn=page ↑↓=line n/u=search`

### Examples

```
sys log --file access.log                       # view an nginx access log (mid-line timestamps recognized)
sys log --file app.log --level error            # ERROR only (warn/error/fatal all work)
sys log --file app.log --search "timeout"       # search + line numbers, n/u jump between matches
sys log --file app.log --level warn --line 100  # start after line 100
sys log --file app.log --newer "2024-01-15 10:23:47"   # only entries at/after this time
sys log --file app.log -f                       # follow the file, scrolling
type app.log | sys log --level error            # piped input (plain text, no ANSI)
```

---

## 10. Data query & conversion (`data`)

Query, modify and convert structured data — a data-compatible subset plus extensions (the successor of the old `sys con`, adding TOML / table / jsonl plus function expressions / slices / counting / projection).

Common usage: `sys data [EXPR] [-f FILE | -u URL] [-i FMT] [-o FMT] [-w FILE] [--compact] [--root] [--depth [N]] [--limit N] [--header|--no-header]`. **Omitting EXPR means pure format conversion** (e.g. `sys data -i yaml -o json < in.yaml`).

### 10.1 Input sources & formats

- **Input sources**: a stdin pipe (format auto-detected) → `-f FILE` (format from the file extension, `-i` overrides) → `-u URL` (GET fetch, default json, `-i` overrides, e.g. `sys data -u https://api.dev/config status`); pick one — `-f` and `-u` are mutually exclusive
- **Seven formats convert into each other**: `json` / `jsonl` (JSON Lines; `.ndjson` counts too) / `yaml` (`.yml`; tab indentation is normalized) / `toml` / `csv` / `md` / `table`. `-i` sets the input format and `-o` the output (default = the input format); unknown input is content-detected (whole-document JSON → per-line JSON → YAML → comma table → table)
- **JSONL semantics**: one independent JSON value per line, parsed into a **root array** — selectors / filter / `--limit` all work array-style; `.log` files fall back to content detection, so no explicit `-i` is needed (e.g. `sys data -f app.log 'level=="error"'`)
- **CSV & table headers**: the first line is the column header by default (columns can be queried by name); a second line of `----` separators is recognized as a header underline and skipped (e.g. the `-------` row of ps output); `--header` forces the first line to be the header, `--no-header` explicitly enables `col1..colN` (never auto-inferred); in a table the extra cells of a row are folded into the last column (a COMMAND containing spaces) and missing cells are padded, with cell types inferred (number / bool / empty / string)
- **Non-UTF-8 auto-decoding**: files and stdin are read as bytes and decoded leniently — UTF-8 BOM (stripped) → UTF-16 LE/BE BOM → strict UTF-8 → GBK → GB18030 → lossy fallback; GBK device logs can be read directly (the trap of "GBK that also happens to be valid UTF-8", e.g. 专业, is corrected by a round-trip read-back)
- **TTY colouring**: on screen json / yaml are colourized jq-style (keys blue, values green, structural characters unchanged); pipes / redirection / `-w` always produce plain text and file contents never contain ANSI; the `NO_COLOR` environment variable turns it off

### 10.2 Selectors & slices

- **Selector syntax**: dotted paths `a.b` (a leading `.` may be omitted), array indexes `list[0]` / `list[-1]` (negative counts from the end), wildcard `list[*]`, quoted keys `m["中文键"]` (spaces / dots / CJK key names), root arrays `[0]` / `[-1]` / `[*]` (e.g. `ps | sys data -i table '[0].PID'`)
- **Python-style list slices (read-only)**: `list[:2]` (first 2) / `list[1:3]` / `list[1:]` / `list[-2:]` (last 2) / `list[::2]` (every other one) / `list[::-1]` (reversed); omitted bounds take their defaults, negative indexes count from the end, the step may be negative and step 0 is an error; out-of-range bounds are clamped, empty ranges are legal and `[0:99]` means everything. **Read-only**: they work in plain queries, in `|count()` pipelines (`data[:2]|count()` = 2) and as a projection left-hand side (`data[1:3]|{id,name}`); `PATH = ...` writes and filter / search / remove paths reject them
- Multi-segment combination: `data[1:3][0]` indexes into the slice result

### 10.3 Modification expressions (assignment)

- `PATH = VALUE`: a JSON literal (number / bool / object / array) or plain text; **`= null` deletes** the key / element
- **`PATH = @FILE`**: read a file as the assigned value — the format comes from the extension (json / jsonl / yaml / toml / csv / md), unknown extensions fall back to JSON → YAML → plain text, e.g. `cat 4k.json | sys data 'wallpaper=@file.json' --root`
- **`PATH = strenv(NAME)`**: read a system environment variable as a string value; an unset variable is an explicit error, e.g. `cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root`
- Arithmetic updates: `PATH += N` / `-=` / `*=` / `/=` for numeric operations (`+=` appends to strings)
- **`-w FILE` write-back**: the whole modified document is written to the file (with an expression + `--root`, stdout stays quiet); **`--root`** prints the whole document after the change instead of the selected value; **`--compact`** prints single-line JSON

### 10.4 Function expressions & predicates

- **`path.filter(pred)`**: keep the array elements satisfying the predicate (read-only, outputs the resulting array); **`path.remove(pred)`**: delete the matching array elements in place (`--root` prints the whole modified document, otherwise the remaining array is printed); **`search(pred)`**: recursively collect every matching node from the root (or from a given node) — `$key` is the object key / array index
- **String / key functions**: `name.contains("x")` / `startsWith` / `endsWith` / `indexOf` / `has("k")` (existence check on object keys / array indexes)
- **`count()` counts nodes**: the number of matched nodes — pipeline forms `data|count()` / `data.filter(...)|count()` / `search(...)|count()` (an array counts its elements, a single value counts 1, no match counts 0), the dot form `data.count()`, and a bare `count()` counts the whole document (an array → its length). E.g. `ps | sys data -i table '[*]|count()'` counts rows
- **Bare predicates auto-filter**: with csv / table / jsonl input, a **comparison expression or a method predicate works without `[*].filter`** — `ps | sys data -i table 'Id==7360'`, `sys data -f app.log 'level=="error"'`, `'Name.contains("x")'` are equivalent to `[*].filter(...)`; on an object-root document a method predicate stays a scalar string function
- **jq-style key projection pipeline**: `data[*]|{id,name,pricing}` — keep only the listed keys of every matched element (missing keys → null); a single match emits one object, several emit an array. The left-hand side may be a filter / search result (its array is spread per element), e.g. `sys data 'clients.filter(name==''modelscope'')|{id,name}' -f config.yaml`. **A bare `{k1,k2}` (no path) acts on the root document directly** — an object root keeps only those keys (jq `. | {k1,k2}`, e.g. `cat 4k.json | sys data '{wallpaper,warningText}'`), an array root projects every row (same as `[*]`, e.g. `ps | sys data -i table '{Id,Name}'`). The key-list validation is shared with the pipeline form, so illegal keys (`{a.b}` nested paths / `{x: y}` aliases / an empty list) error out the same way. **A trailing projection on a deep node needs no pipe**: `sites[0]{key,name}` / `sites[0].{key,name}` == `sites[0]|{key,name}`, and a slice works as a left-hand side too (`sites[:2]{key,name}`)
- **Predicate language** (data-compatible): `$this` / `$key`, field paths (`name`, `a.b[0]`, `m["中文键"]`), literals (numbers / single- or double-quoted strings / true / false / null), `== != > >= < <= && || !`, and function calls (`contains("x")` / `startsWith` / `endsWith` / `indexOf` / `has("k")`), e.g. `sys data 'sites.filter(type==1)'`

### 10.5 Document-level options

- **`--depth N`** (default 1; a bare `--depth` means 1): removes every subtree deeper than N levels (root = level 0, levels 0..=N are kept) and is applied before the expression — `sys data --depth 1 -f cfg.yaml` keeps only the top-level keys
- **`--limit N`**: when the document root is an array only the first N elements are kept — with csv / table that means "only the first N data rows, the header is still printed"; jsonl behaves the same; `--limit 0` prints nothing (head -n 0 semantics). It is applied before the expression, so queries / filters run on the truncated document; an object root is an explicit error
- `-f` / `-u` / `-i` / `-o` / `-w` / `--header` / `--no-header` / `--compact` / `--root` combine freely; `--depth` and `--limit` are compatible with selectors / assignments / functions / projections

### 10.6 Examples

```
echo '{"a":{"b":"hello"}}' | sys data 'a.b'            # read (stdin auto-detected as json)
sys data -f cfg.yaml 'server.port = 8080' -w cfg.yaml # modify and write back
sys data -f cfg.yaml 'server.port = 8080' --root      # modify and print the whole document
sys data -i json -o yaml '{}'                          # format conversion
sys data -i yaml -o json < in.yaml                     # YAML -> JSON
sys data -f app.log 'level=="error"'                  # bare predicate on a jsonl log
sys data 'sites.filter(type==1)'                       # filter an array
sys data 'sites.remove(name.contains("web"))' --root   # delete elements
sys data 'search(has("name"))'                         # recursive search
sys data 'name.endsWith("server")'                     # string function
sys data 'data[:2]|count()'                            # slice + count (= 2)
ps | sys data -i table '[*]|count()'                   # count rows
ps | sys data -i table '[0].PID'                       # root array: a field of the first element
ps | sys data -i table 'Id==7360'                      # bare predicate auto-filter (same as filter)
sys data 'data[*]|{id,name}' -f config.yaml           # jq-style key projection
cat 4k.json | sys data '{wallpaper,warningText}'       # bare projection: an object root keeps only those keys
ps | sys data -i table '{Id,Name}'                     # bare projection: an array root projects every row
sys data -f 4k.json 'sites[0]{key,name}'               # trailing projection on a deep node
sys data -f 4k.json 'sites[0].{key,name,type}'         # the dotted form is equivalent
sys data -f 4k.json 'sites[:2]{key,name}'              # slice + trailing projection: the first 2 sites
cat 4k.json | sys data 'wallpaper=@file.json' --root   # assign a file
cat 4k.json | sys data 'wallpaper=strenv(ROOT)' --root  # assign an environment variable
sys data -u https://api.dev/config status              # fetch from a URL + query
sys data -i table --limit 3                            # only the first 3 rows (header kept)
sys data --depth 1 -f cfg.yaml                         # keep only the top-level keys
sys data -f app.log '[*].filter(level=="error")' --limit 10  # filter + truncate
```

---

## Limitations

- `http`: `--auth-type digest`, `--session`, `--stream`, `--ssl`, `--cert`, custom `--boundary` are not implemented (see `sys help http`)
- `path export` `.reg` files are Windows-only (Registry Editor format); use `.txt` / `.json` for backups on Linux
- `link` symlinks on Windows require Developer Mode or admin rights; falls back to copy automatically
- `ai` uses models.dev's public endpoint; offline it serves the 24 h cache, and reports an error when the cache is stale and there is no network

## Development & verification

- `cargo test`: 447 tests pass (platform-specific cases gated by `cfg`)
- `cargo check`, `cargo test`, `cargo build --release` and end-to-end smoke tests have been run on both Windows (msvc) and Ubuntu (real WSL)
- End-to-end coverage: PATH CRUD & registry restore, link fallback chain, env persistence/temporary mode, full http flag smoke tests, live models.dev fetch with canonical preference
- `dev/` ships local smoke tooling: `test_server.ps1` (HttpListener test server on port 18899), `smoke_http.ps1` / `smoke_http2.ps1`, `test_body.json`

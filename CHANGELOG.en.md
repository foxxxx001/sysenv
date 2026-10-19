# sys Changelog

Versioned record of new features / fixes / release conventions. Artifact naming: `sys-<platform>-<arch>_v<version>`, release binaries compressed with UPX (see README "Build & Release").

## v0.6.1 (2026-10-19)

### Changed
- **CSV / table header detection fixed**: the first line is the column header by default; a second line made of `----` separators (e.g. ps' `------- ------`) is recognised as a header underline and skipped, so it no longer becomes a junk data row; `--no-header` is the only way to enable `col1..colN` (no more auto-inference), `--header` forces the first line as header
- version 0.6.0 -> 0.6.1; README (zh / en) data row updated
- `cargo test --bin sys` 319 -> 321 all green

---

## v0.6.0 (2026-10-19)

### Changed
- **CSV / table column headers virtualised as object keys**: `-i csv` header handling now matches table — a first non-blank row with any non-numeric cell is the header (column titles become keys), an all-numeric first row is data and columns are auto-named `col1..colN`; `--header` / `--no-header` now apply to `-i csv` too; empty header cells fall back to `colN`
- version 0.5.13 -> 0.6.0; README (zh / en) data row updated to the unified CSV/TABLE header description
- `cargo test --bin sys` 315 -> 319 all green

---

## v0.5.13 (2026-10-19)

### Added
- **Root-array function expressions**: a bare `[*]` addresses the array itself when the root is an array, so `sys data -i table '[*].filter(Id==7360)'` / `'[*].search(...)'` filter / search the whole table (filter / search share `resolve_single`, both fixed)

### Changed
- version 0.5.12 -> 0.5.13; README (zh / en) data row notes root-array `[*].filter(...)`
- `cargo test --bin sys` 314 -> 315 all green

---

## v0.5.12 (2026-10-19)

### Added
- **`sys data -i table`**: parse whitespace-separated tabular output (e.g. `ps`), one record per line; a first non-blank line with any non-numeric cell is treated as the header (queryable by column name, e.g. `[0].PID` / `[*].COMMAND`), otherwise columns are named `col1..colN`; `--header` / `--no-header` force either mode (mutually exclusive); extra cells fold into the last column, missing cells pad empty, cell types inferred like CSV
- **`-o table`**: aligned table output for arrays of objects / a single object / arrays of scalars; scalar reads (e.g. `[0].PID`) print the number / bool / string directly
- **Root-array selectors**: `[0]` / `[-1]` / `[*]` are now valid selector prefixes addressing a root array

### Changed
- version 0.5.11 -> 0.5.12; README (zh / en) data row updated to six formats + table parsing
- `cargo test --bin sys` 300 -> 314 all green

---

## v0.5.11 (2026-10-17)

### Removed
- **`sys search` subcommand deleted** (web search via the Bocha AI Web/AI Search API). Removed `Cmd::Search` / `SearchArgs` / dispatch and help text; chat.rs dropped `Config.search` and `SearchKey`, the config `search:` section parsing, all Bocha code (10 `bocha_*` functions + 2 endpoint consts), `cmd_search` and 8 tests; short.rs removed the `ssearch` shim (8 → 7)
- Installed `ssearch.cmd` shims are not actively cleaned up (release convention does not touch `sys short`); they disappear after re-running `sys short`

### Changed
- version 0.5.10 -> 0.5.11; README (zh / en): search feature row, the `5.6/5.3 Web search` section and the `ssearch` shim-table row removed (following sections renumbered; shim docs updated to seven commands, example count 6 → 7)
- `cargo test --bin sys` 308 -> 300 all green

---

## v0.5.10 (2026-10-17)

### Removed
- **`sys con` subcommand deleted**: its format conversion is fully covered by `sys data` (`dasel`) — JSON / YAML / TOML / CSV / MD via `-i` / `-o` / `-f` / `-w`. Removed `Cmd::Con` / `ConArgs` / dispatch and help text; convert.rs dropped the con-only code (`Format` / `detect_format` / `parse` / `parse_yaml_value` / `parse_args` / `CON_HELP` / `cmd_con`) and 4 tests while **keeping** the dasel-shared parse / serialize helpers (module docs rewritten); short.rs removed the `scon` shim (9 → 8)
- Installed `scon.cmd` shims are not actively cleaned up (release convention does not touch `sys short`); they disappear after re-running `sys short`

### Changed
- version 0.5.9 -> 0.5.10; README (zh / en): con feature row, the `5.7/5.8 Format conversion` section and the `scon` shim-table row removed (shim docs updated to eight commands + no shims for data/log)
- `cargo test --bin sys` 312 -> 308 all green

---

## v0.5.9 (2026-10-17)

### Added
- **`sys log` — colored less-style log viewer**: parses common log formats and renders them with colors; keys behave like less (PageUp / PageDown page, ↑ / ↓ line, Home / End / g / G jump, q / Esc / Ctrl-C quit)
  - **Timestamp parsing (common formats, recognized as the start of each entry and rendered orange RGB(255,165,0))**: ISO `2024-01-15 10:23:45`, `2024-01-15T10:23:45.123Z`, Log4j comma-ms `2024-01-15 10:23:45,123`; slash `2024/01/15 10:23:46`; nginx access log `15/Jan/2024:10:23:49 +0800` (timestamp in the middle of the line is recognized too — first 80 bytes scanned, prefix `127.0.0.1 - - [` kept intact); syslog `Jan 15 10:23:48` (no year → current year, falling back to the previous year when >1 day in the future); US `01/15/2024`
  - **Level gradient colors**: trace (178,230,178) / debug (102,204,102) / info (0,180,0) are green (info and below, lighter = lower), warning (255,90,90) / error (204,0,0) / fatal (140,0,0, bold) are red
  - **App markers**: tokens right after the timestamp such as `[http-nio-8080-exec-1]`, `app[1234]`, `[01-node-5]` render dark yellow RGB(184,134,11)
  - **`--file FILE`**: log file to parse (default: stdin; `-` also means stdin)
  - **`--level LEVEL`**: filter by level (trace | debug | info | warn | warning | error | fatal; warn == warning, err == error, critical/crit/alert/emerg == fatal; lines without a level are dropped)
  - **`--search KEYWORD`**: case-insensitive keyword search — line numbers shown (dim gray), matches reverse-highlighted; in the interactive view **n jumps to the next match / u to the previous** (wraps around, match centered)
  - **`--line N`**: parse only from line N onward (original line numbers kept)
  - **`--newer TIME`**: show only entries at/after the given time (continuation lines without timestamps are kept)
  - **`-f`**: follow the file — 150ms polling incremental reads, truncation / rotation auto-re-read, scrolling display; files only (stdin errors with exit code 1)
  - Non-TTY (pipe / redirect) output is plain text without ANSI; on a TTY it enters a crossterm full-screen interactive view with a status bar (file name / line range / level filter / search term & match count / follow state / key hints)
  - New dependency crossterm 0.28; 29 new `log::tests` unit tests; `cargo test --bin sys` 312 all green

### Changed
- version 0.5.8 -> 0.5.9; README (zh / en) adds a `sys log` section with examples
- `src/convert.rs`: `parse` / `yval_to_json` / `parse_csv_rows` / `cell_to_value` / `parse_md_row` promoted to `pub(crate)` (required by v0.5.8's dasel.rs; committed with this release)

### Smoke-tested (real binary, Windows)
- `sys log --file dev/sample.log`: 8 plain-text lines fully reconstructed (nginx in-line prefix included); `--level warn` keeps only the WARN line; `--search app` shows all 8 lines with line numbers; `--newer "2024-01-15 10:23:47"` filters correctly; `--line 6 --search GET` correct; invalid `--level bogus` / `--newer garbage` print a friendly error with exit code 1; stdin piped with `-f` errors with exit code 1
- Real-console TTY smoke: the viewer enters raw mode / full-screen rendering without crashing (process stays alive waiting for keys)

---

## v0.5.8 (2026-10-17)

### Added
- **`sys data` function expressions (dasel-compatible)**: `list.filter(predicate)` keeps matching array elements; `list.remove(predicate)` deletes the matching elements in place (each deleted element is the parent node containing the matched content); `search(predicate)` recursively collects matching nodes; string / key functions `name.endsWith("x")` (contains / startsWith / endsWith / indexOf / has)
- **Predicate language**: `$this` / `$key` (current node / key or index), field paths (`name`, `a.b[0]`, `m["中文键"]`), literals (numbers, quoted strings, true / false / null), comparisons & logic (`== != > >= < <=`, `&&`, `||`, `!`), function calls (`contains("x")` / `startsWith("x")` / `endsWith("x")` / `indexOf("x")` / `has("key")` / `has(0)`); single-quoted strings are also accepted (PowerShell-friendly)
- **`sys data --depth N`**: removes every node deeper than N levels (root = level 0; levels 0..=N are kept, deeper subtrees are removed entirely); bare `--depth` means 1; applied before the expression
- **Assignment detection fix**: `split_expression` now skips the `=` inside `==` / `!=` / `>=` / `<=` comparison operators, so function expressions are no longer mis-split as assignments
- 7 new unit tests (function-expr parsing / predicate eval / filter / remove / search / string functions / depth pruning); `cargo test` 283 all green

### Removed
- **`sys ai task` module deleted**: the Task struct, `tasks` config parsing, all task tools (task_tools / route_tasks / built-in search sources / score matching) and their tests removed from chat.rs; the task field, `AiCmd::Task` variant and dispatch branch removed from ai.rs / main.rs; `sys ai --help` no longer lists task

### Changed
- README (zh / en): the `ai task` section and feature-table row removed, data function-expression / `--depth` docs added; version 0.5.7 -> 0.5.8

---

## v0.5.0 (2026-10-12)

### Changed
- **`sys ai config` default export = the models `sys ai chat` uses**: without `-m` (and without the positional `SELECT`), the export no longer covers every model — it now matches `sys ai chat` by resolving the config's top-level `model:` selector (comma-separated lists included), falling back to the first provider's first model when none is configured; explicit `-m` / positional selectors behave as before
- version 0.4.72 -> 0.5.0; `cargo test` 256 -> 257 all green

---

## v0.4.72 (2026-10-12)

### Added
- **`sys ai config` adds codebuddy / workbuddy / trae export formats**:
  - **`codebuddy`** (`~/.codebuddy/models.json`) and **`workbuddy`** (`~/.workbuddy/models.json`): share the models.json schema, one entry per model with `id` / `name` / `vendor` / `url` (`{api_base}/chat/completions`) / inline `apiKey`; `maxInputTokens` / `maxOutputTokens` / `supportsImages` are added when the config has `max_input_tokens` / `max_tokens` / a `type` containing `image`; duplicate model ids are deduped (first provider wins) and `availableModels` stays in sync
  - **`trae`** (trae_cli.yaml): YAML `models` list, OpenAI-compatible providers use the `open_ai` block and anthropic the `claude` block, deduped by model id; default path Windows `%USERPROFILE%\AppData\Roaming\trae_cli\trae_cli.yaml`, Linux `$XDG_CONFIG_HOME` (or `~/.config/trae_cli/trae_cli.yaml`)
- **`-m/--models` multi-model selection**: exactly the `sys ai chat -m` format (`provider:model` / `provider:*` / bare model name, half-/full-width comma separated, case-insensitive, overlapping selections deduped) via `chat::resolve_targets`; the legacy positional `SELECT` stays as a single selector (mutually exclusive with `-m`)
- **`--update` in-place merge**: finds the software's default config file for the format (`-f FILE` overrides the path); a missing file prints `config file not found ... nothing to update`; an existing file is first backed up to `.bak-YYYYMMDD-HHMMSS` (never overwriting an older backup), then the missing selected models are merged in (existing entries and other top-level keys kept), and success reports `added N model(s): ...` (or `nothing to add`); supported only for codebuddy / workbuddy / trae (other formats error clearly); mutually exclusive with `--show`
- README / CHANGELOG updated; version 0.4.71 -> 0.4.72

### Fixed
- **UTF-8 BOM stripped before merge parsing**: models.json / trae_cli.yaml saved by Windows editors often carry a BOM, which breaks strict JSON / YAML parsing (`expected value at line 1 column 1`); the content is `strip_prefix('\u{feff}')`-ed before parsing
- 9 new unit tests (chat-format multi-select / dedup, codebuddy/trae export shape and dedup, models.json merge and second-merge no-op, trae YAML merge keeping other keys, missing-file prompt, unsupported-format error, BOM tolerance, default-path mapping); `cargo test` 247 -> 256 all green

---

## v0.4.71 (2026-10-12)

### Added
- **`sys ai model --src openrouter`**: new `--src` flag (default `models`, unchanged behavior when omitted); with `--src openrouter` it HTTP-GETs `https://openrouter.ai/api/v1/models` and lists each model's `id` / `name` / `context_length` / price (`pricing.prompt`, per 1M tokens) / creation time (`created` unix timestamp shown as local `YYYY-MM-DD HH:MM:SS`); **model-name filtering supported**: `sys ai model deepseek --src openrouter` / `-s QUERY` filter by name (exact first, then substring; comma-separated terms are OR-ed); without a name the full list is shown; supports `--limit N` / `--json` / `-o json|csv|yaml|md`
- **`sys ai price` standalone command (fully replaces `info price`)**: `sys ai price P1,P2,...` queries models.dev prices (`--refresh`, text table — same behavior as the old `info price`); new `--src` flag (default `models`); with `--src openrouter` it queries openrouter.ai `/api/v1/models` and shows each model's `pricing.prompt` / `pricing.completion` prices (per 1M tokens, USD), with an **optional positional model-name filter** (`sys ai price deepseek --src openrouter`); no filter shows the full list; supports `--limit` / `--json` / `-o json|csv|yaml|md`
- **Removed `sys ai info price`**: duplicated `sys ai price`; `info` no longer accepts the `price` field (the error points to `sys ai price`)
- README.md / README.en.md updated; version 0.4.70 -> 0.4.71; `cargo test` 241 tests all green

---

## v0.4.39 (2026-10-09)

### Changed
- **`sys ai chat --update`: explicitly fill missing capability fields and write them back to the config.** Without the flag the v0.4.38 decoupled behavior stays (zero queries, zero write-backs). With it, before sending, the target models' **missing** `max_input_tokens` / `type` are looked up in models.dev (24 h cache) and persisted into the config file — best effort (lookup / write failures only warn and never block); models whose fields are already set are not queried; stderr prints what was actually filled (e.g. `sys: update: filled max_input_tokens=..., type=... for ...`)
- Restored `lookup_model_capabilities` in ai.rs (exact match first, then fuzzy; failures yield empty) for `--update`; `Model.type` parsing restored (needed to tell whether `type` is missing — existing `type: text,image` lines become effective again)
- Extracted a unit-testable `missing_capability_fields` decision helper with a new test; the `cmd_task` path does not accept `--update` and stays zero-query
- `config.example.yaml` and README 5.2 / feature table updated accordingly
- Version 0.4.38 -> 0.4.39

## v0.4.38 (2026-10-09)

### Changed
- **`sys ai chat` / `sys ai task` fully decoupled from models.dev**: the pre-request "capability auto-fill" (`ensure_model_capabilities`) is removed — when a model lacks `max_input_tokens` / `type`, chat no longer queries models.dev (network or 24 h cache) and no longer writes back to the config; capability fields now come only from the config file (absent `max_input_tokens` = no truncation, absent `type` does not affect the request)
- Removed `lookup_model_capabilities` from ai.rs (no longer used); `Model.type` (the per-model modality field) had no consumers left and was removed too — existing `type: text,image` lines in configs stay harmless and are ignored by the parser
- `config.example.yaml` and README 5.2 updated accordingly
- Version 0.4.37 -> 0.4.38

## v0.4.37 (2026-10-09)

### Changed
- **Removed the `sys env config` subcommand** (added in v0.4.36, out of scope): the `--show` / backup / report behavior of `ai config` now lives only under `sys ai config`; `sys env` is back to get / set / unset / list
- **`sys ai config` finalized**:
  - `--show` preview: prints the target software's config file path first, then the config that would be applied (no file is touched); when `-f FILE` is also given, an extra `write target` line is printed
  - `-f FILE` apply report now prints **provider count + model count + target path** (previously models only); backup info unchanged
  - `backup_file` appends a `-N` suffix on name collisions, so repeated backups within the same second never overwrite each other
  - Added deterministic unit tests for `software_config_path_in` / `backup_file` (format mapping, backup naming and content preservation)
- Version 0.4.36 -> 0.4.37

## v0.4.36 (2026-10-09)

### Added
- **`sys env config` subcommand**: same arguments and implementation as `sys ai config`, for inspecting / applying software config exports (`codex` | `opencode` | `litellm` | `freellmapi`):
  - **`--show`**: prints the target software's config file path first (codex → `~/.codex/config.toml`, opencode → `~/.config/opencode/opencode.json`, litellm → `~/.litellm/config.yaml`, freellmapi → `freellmapi.config.json` in the current directory; a missing default path prompts `-f FILE`), then prints the config that would be applied to that software — no file is touched
  - **`-f FILE` apply mode**: before writing, the existing config file is backed up to `FILE.bak-YYYYMMDD-HHMMSS` (same directory); after writing, the actual change is reported — exported model count / target file path / backup file path; when the target file did not exist, it reports "created a new config file"
  - `sys ai config` gained the same `--show` and backup behavior; both entries stay consistent
- Version 0.4.35 -> 0.4.36

## v0.4.35 (2026-10-09)

### Changed
- **`sys ai info list-model` text view shows `CREATED` as local date-time**: the unix-second timestamp returned by OpenAI-compatible providers is now displayed as local `YYYY-MM-DD HH:MM:SS` (system timezone) in the text view; Anthropic ISO `created_at` and empty / non-numeric values pass through unchanged; `-o json` / `-o csv` keep the API's raw timestamp (machine-readable). Added a pure `fmt_created` helper with a round-trip unit test
- Version 0.4.34 -> 0.4.35

## v0.4.34 (2026-10-09)

### Added
- **`sys ai info` gained `list-model PROVIDER`**: queries the provider's own models API for the models its configured api_key can actually use — OpenAI-compatible providers hit `GET {api_base}/models` (Bearer api_key), anthropic providers hit `GET {api_base}/v1/models` (`{api_base}/models` when api_base already ends with `/v1`; x-api-key / anthropic-version); Anthropic `next_page` pagination is followed automatically (up to 20 pages). The text view prints an aligned `MODEL / OWNED_BY / CREATED` table (OpenAI `owned_by` / `created` and Anthropic `display_name` / `created_at` are normalized into the same columns); `-o json` / `-o csv` / `--json` print a JSON array or CSV table (fields `id` / `owned_by` / `created`). Always live, no cache
- Version 0.4.33 -> 0.4.34

## v0.4.33 (2026-10-08)

### Added
- **`sys ai model --price PRICE`**: keeps only models whose `cost.input` and `cost.output` are both <= PRICE (USD per 1M tokens; a missing cost counts as 0); optional, defaults to 0 = free / unpriced models only; combines with `--provider` / `--model-type` / `--date` / `--open` and name search; the text view footer and the no-match hint annotate the price condition
- Version 0.4.32 -> 0.4.33

## v0.4.32 (2026-10-08)

### Added
- **`sys ai model --provider PROVIDER`**: keeps only models whose provider id or name contains the value (case-insensitive substring, e.g. `--provider openai` / `--provider 阿里`), stacks with the `--model-type` / `--date` / `--open` filters; works with the no-arg list, `NAME` and `--search`; the text view footer and the no-match hint annotate the provider filter
- Version 0.4.31 -> 0.4.32

## v0.4.31 (2026-10-07)

### Added
- **`sfile` / `scon` shortcut shims for `sys file` / `sys con`**: `sys short` now installs nine short commands (spath/senv/slink/shttp/sai/ssearch/stask/sfile/scon); `sfile` ≡ `sys file` (text search/replace/file view), `scon` ≡ `sys con` (json/csv/md/yaml conversion)
- Version 0.4.30 -> 0.4.31

## v0.4.30 (2026-10-07)

### Added
- **`sys ai info provider` gained `-o yaml`, now the default format**: without `-o` it prints a YAML list (one `- name:` entry per provider with api_base / api_key / docs / console); `-o yaml` is the explicit equivalent; `-o json` / `--json` and `-o csv` are unchanged; the other `-o` commands (`ai model` / `ai provider` / `ai cn-model` / `ai info` no-field & model) also accept yaml now
- Version 0.4.29 -> 0.4.30

## v0.4.29 (2026-10-07)

### Changed
- **`sys ai server --help` grouped lines slimmed down**: the provider line keeps only name / api_base (api_key / docs / console removed), still followed by that provider's indented model lines (`{provider}:{name}`)
- Version 0.4.28 -> 0.4.29

## v0.4.28 (2026-10-07)

### Changed
- **`sys ai server --help` prints the config grouped by provider**: one line per provider (name / api_base / api_key / docs / console) followed by that provider's indented model lines (`{provider}:{name}`), replacing the previous provider-table + separate model-list layout
- Version 0.4.27 -> 0.4.28

## v0.4.27 (2026-10-07)

### Changed
- **`sys ai chat --server` became the `sys ai server` subcommand**: starting the server moved from `sys ai chat --server [ADDR]` to `sys ai server [ADDR]` (default `127.0.0.1:10000`, a bare port/IP keeps the other default, new `-c/--config` selects the config file); `sys ai chat --server --help` moved to `sys ai server --help` (probes whether a server is already running — running prints the address/port, not-running says `server is NOT running`, both then print the configured provider table and model list); `chat` dropped `--server` / the custom `--help` and its default clap help is back
- Version 0.4.26 -> 0.4.27

## v0.4.26 (2026-10-07)

### Added
- **`sys ai chat --server --help` server-status query**: probes the address (GET /v1/models, 2 s timeout; bind-all hosts `0.0.0.0` / `::` are probed via 127.0.0.1) to tell whether the server is already running — running prints the current address/port, not-running says `server is NOT running` with the start command; both cases then print the configured provider table (name/api_base/api_key/DOCS/CONSOLE) and the full model list (`{provider}:{name}`). The chat subcommand now owns a custom `-h/--help` (without `--server` it prints a usage summary)
- Version 0.4.25 -> 0.4.26

## v0.4.25 (2026-10-07)

### Added
- **`sys ai info` default combined output**: with no FIELD argument it prints the configured provider table (name / api_base / api_key / DOCS / CONSOLE) plus the full model list (`{provider}:{name}`) together; `-o json` emits one object `{"providers": [...], "models": [...]}`; no-field `-o csv` gives a clear hint to use `info provider` / `info model` separately
- Version 0.4.24 -> 0.4.25

## v0.4.24 (2026-10-07)

### Added
- **`sys ai config` multi-format config export**: exports the local config's `clients` section as `codex` (TOML, one `[model_providers]` table per provider + `env_key` + stderr `export` hints), `opencode` (JSON, `@ai-sdk/openai-compatible` + inline apiKey), `litellm` (YAML `model_list`, exposed name `{provider}:{model}` to avoid collisions) and `freellmapi` (JSON `customProviders`); default exports every provider's every model, `provider:*` exports one provider's models, `provider:model` exports one model (clear errors listing the available names on no match); `-f/--file` writes to a file (stdout by default), `-c/--config` selects the config file; a missing config keeps the existing explicit error
- **`sys ai chat --server [ADDR]` OpenAI-compatible local server**: serves the configured providers as an OpenAI-format API, default `127.0.0.1:10000`; a bare port keeps the default host, a bare IP keeps the default port; exposes `POST /v1/chat/completions` (non-streaming passthrough + SSE streaming via chunked transfer) and `GET /v1/models` (all models, id `{provider}:{model}`); request `model` accepts `provider:model` / `provider:*` / bare name / `auto` (= config default); only `type: openai` providers are served (anthropic targets get a clear error); unknown models return 404 listing the available names
- Version 0.4.23 -> 0.4.24

## v0.4.23 (2026-10-07)

### Changed
- **Project-wide rename `sysenv` → `sys`**: executable name (`sysenv.exe` → `sys.exe`), command output prefixes (`sysenv: ...` errors/warnings), `--version`/`--help` display, User-Agent, shim comments, doc command examples and artifact naming convention (`sysenv-windows-...` → `sys-windows-...`) all use `sys` now.
- **User data dir kept as-is**: `~/.sysenv/config.yaml`, `~/.sysenv/bin` managed dir and `~/.sysenv/cache` are **not migrated**; the renamed binary keeps reading/writing the existing data.
- **Ephemeral cache dir renamed**: Windows `%LOCALAPPDATA%\sysenv\` → `%LOCALAPPDATA%\sys\`, Linux `~/.cache/sysenv/` → `~/.cache/sys/` (regenerable; first run re-fetches automatically).
- Version 0.4.22 -> 0.4.23

## v0.4.11 (2026-10-05)

### Changed
- **Search sources slimmed down**: removed `zhihu` / `baidu` / `bing` / `dxtower` / `cls` (CLS finance) / `penalty` / `company` / `tophub` — 8 sources gone, the generic aggregate source `sogou` stays; `ai task --list-source` now lists **11 sources** (bilibili / github / hn / toutiao / oschina / smzdm / sogou / enlightent / dongchedi / autohome / szhousing).
- **Tasks slimmed down**: removed `penalty` / `company` / `finance` / `technews` tasks; `ai task` now has **8 tasks** (weather / cloudrank / hotnews / auto / cartech / oschina / deals / szhousing).
- **szhousing rewritten (plain HTTP)**: dropped the Chrome/CDP browser approach and queries the official API directly — the platform API (`/szfdcscjy/*`) is measured to bypass the RiverSecurity challenge (it guards only the HTML page), so a POST with a regular Chrome UA + Origin + Referer headers returns 200; **no browser and no dependency on any local software**, identical on Windows/Ubuntu; added per-building **filing price average**; single query dropped from ~5s to <1s.
- `ai cn-model` gained a `--list` flag: lists all 1015 models (`--limit` truncates), and the no-argument error now reads `provide a model NAME, --search QUERY, or --list`.

## v0.4.10 (2026-10-05)

### Fixed
- **hotdrama / cloudrank search failure**: fixed off-topic tool results for requests like "今天电视剧热度排行榜".
  - Root cause: vendor names like 德塔文/云合 get tokenized badly by the search engines (e.g. 德塔文 → 德), returning Chinese-character dictionary entries, Amap links and other irrelevant rows; the Data Eye site (dxtower.com) is also unreachable directly.
  - Fix: the `dxtower` source now uses the generic query `电视剧 热度 排行榜 今日` and `enlightent` uses `电视剧 热播 榜单 今日`, matching hot-drama channel pages on iQiyi / Tencent Video / CCTV, which the model summarizes into today's drama ranks.
  - hotdrama / cloudrank task descriptions updated to "电视剧热度排行榜/热播榜（AI 聚合多源搜索）".
- Smoke-verified: `-t hotdrama` returns real platform hot-drama channel results with no dictionary/encyclopedia/map pollution.

## v0.4.9 (2026-10-05)

### Fixed
- **AI aggregate-search result cleanup**: fixed results being flooded with low-value encyclopedia entries (Baidu Baike etc.) which degraded search quality.
  - New `is_low_value_row` filter drops rows whose title contains "百度百科/搜狗百科/维基百科/互动百科" or whose URL matches `baike.baidu.com` / `zh|en.wikipedia.org` / `baike.sogou.com` / `baike.com` (the title channel covers Sogou `/link?url=` redirects where the real domain is hidden).
  - A failed Sogou request no longer aborts the task; it falls back to Bing. When neither engine yields usable results, the raw deduplicated results are returned as a last resort so the task never fails on over-filtering.
- New unit test `low_value_rows_are_detected` (79 tests all green).

## v0.4.8 (2026-10-05)

### Added
- `sys ai task --list-source`: lists all built-in search sources with their **name, kind** (`api` direct HTTP API / `search` AI aggregate search with Sogou first and Bing fallback / `generic` general search engine requiring a `q`/`query` argument), **purpose and access URL** — 19 sources (zhihu/baidu/bilibili/github/hn/toutiao/tophub/oschina/smzdm/bing/sogou/dxtower/enlightent/cls/dongchedi/autohome/szhousing/penalty/company).

## v0.4.7 (2026-10-05)

### Added
- Two new search sources for `sys ai task`:
  - `penalty`: administrative penalty / dishonest-executor records for a person or organization (public disclosure channels, AI aggregate search).
  - `company`: company registration info (legal representative, registered capital, founding date, etc.; public disclosure channels, AI aggregate search).
- Implementation note: Creditchina (creditchina.gov.cn) is unreachable, the enterprise credit system (gsxt.gov.cn) returns 521, and the court execution site / wenshu / aiqicha all require CAPTCHA or JS rendering, so direct fetching is impossible. AI aggregate search (Sogou first, Bing fallback) over public disclosure pages (qcc / aiqicha / baike / government notices) is used instead; the entity name comes from the model's `name` argument, and the model honestly reports when no records are found.
- Manual `-t` tasks now fill the first declared parameter with a bare free-text argument (e.g. `-t company 字节跳动` → `name=字节跳动`), replacing both the `{name}` placeholder in the msg and the tool query. Existing tasks like `weather` are unaffected.
- Sample config adds the `penalty` / `company` tasks (both `~/.sysenv/config.yaml` and `doc/config.yaml`).

## v0.4.6 (2026-10-05)

### Added
- New search source `szhousing` for `sys ai task`: **Shenzhen housing sales** (new/second-hand transaction counts from the Shenzhen Real Estate Information Platform's public data).
- Implementation note: `fdc.zjj.sz.gov.cn` runs a Ruishi dynamic WAF (full browser headers still return HTTP 412), so it cannot be fetched directly. It uses AI aggregate search (Sogou first, Bing fallback) over public channels (Leyoujia / Centaline / the housing bureau site) and the model summarizes honestly with sources.
- Sample config adds the `szhousing` task (both `~/.sysenv/config.yaml` and `doc/config.yaml`).

## v0.4.5 (2026-10-05)

### Added
- `sys ai task` gains **10 new built-in search sources** covering the five requested data groups:
  - **Hot TV series**: `dxtower` (Datatower drama climate index), `enlightent` (Enlightent ranking) — implemented as AI aggregate search since the target sites' data endpoints are signed / WAF-gated.
  - **Finance & news media**: `toutiao` (Toutiao hot board, direct JSON), `cls` (CLS.cn telegraph, AI aggregate search).
  - **Auto news**: `autohome`, `dongchedi` — AI aggregate search.
  - **Tech news**: `tophub` (tophub.today developer board, direct SSR), `oschina` (OSChina news, direct SSR).
  - **Deals**: `smzdm` (today's deals, direct SSR, title + price).
- **AI aggregate search engine**: Sogou first (reliable Chinese tokenization) with an automatic Bing fallback when Sogou serves a CAPTCHA page or empty results; the program feeds the fetched headlines back to the model, which filters and summarizes honestly.
- Manual `-t NAME` task selection now **executes the task's `api`/`search` tool and feeds real data back** before replying (previously only function_call auto-routing executed tools).
- Sample config (`~/.sysenv/config.yaml` and `doc/config.yaml`) adds 9 tasks: `hotdrama` / `cloudrank` / `hotnews` / `finance` / `auto` / `cartech` / `technews` / `oschina` / `deals`.

### Implementation notes
- All new sources are pure in-code HTTP requests (reqwest with a browser UA), **no local shell commands**; HTML pages are parsed with built-in helpers (tag stripping, HTML entity decoding, percent-encoding).
- Datatower (dxtower.com) and Enlightent (enlightent.cn) were verified to reject direct connections (WAF / signed ranking APIs); CLS.cn data endpoints are fully signed; Dongchedi/Autohome homepages are JS-rendered. These sites therefore use AI aggregate search: fetch relevant Sogou/Bing results and let the model filter and summarize honestly, stating when data is insufficient.

## v0.4.4 (2026-10-05)

### Added
- `sys ai task` tool execution is now **pure in-code HTTP, with local shell commands banned** (the `tool:` command template was removed and `run_tool_cmd` deleted). A task can declare either of two tools which the program executes after function_call selection and feeds the **real data back** to the model:
  - **`api: <URL template>`** — fixed HTTP API: `{key}` / `{key:default}` placeholders are filled from the model's tool arguments (empty values fall back to the default), and the response body is returned as the tool result; `params: [k1, k2]` declares optional parameters, added to the function schema.
  - **`search: <source>`** — fuzzy AI web search: five built-in sources — `zhihu` (daily news), `baidu` (real-time hot search), `bilibili` (popular videos), `github` (repos created in the last 7 days, optional `date` argument), `hn` (Hacker News) — the program fetches the headline list and feeds it back for the model to filter and summarize.
- The home-grown YAML parser now supports inline arrays `[a, b]` (both `params: [lat, lon]` and block-list forms work).
- Sample config adds `topnews` (zhihu) and `topshow` (baidu) tasks; weather now uses the `api:` form.

### Fixed
- Empty tool-argument strings no longer override `{key:default}` URL placeholders (previously produced invalid URLs).

## v0.4.3 (2026-10-05)

### Added
- `sys ai task` without `-t` and with a user request (arguments or stdin) now uses **function_call auto-routing**:
  - Each task's `name` becomes the function name and `desc` the function description, registered as tools (both OpenAI and Anthropic protocols).
  - The LLM picks the best-matching task via function calls; the user request is carried by the `input` parameter.
  - The selected task is executed automatically: its `desc` plus the user request are assembled into the chat message and the final reply is printed.
  - When the model selects no task, an explicit error suggests using `-t NAME` instead.
- Backward compatible: no input still lists the tasks; `-t NAME` (template placeholder substitution) and `-t *` (0-10 scoring) keep their behavior; routing requests also use dynamic weighted fallback across models.

## v0.4.2 (2026-10-05)

### Added
- `sys http` new **`--file FILE`** flag: reads a local file as the raw request body (equivalent to the positional `@FILE`), auto-defaults to POST; mutually exclusive with the positional `@FILE` / `--raw`, with an explicit error.
- Request-item values starting with `@` (`key=@file`, `key:=@file`, `key==@file`, `key:@file`) read the local file as the field value: **UTF-8 BOM and trailing newlines are stripped automatically**, and special characters such as quotes / backslashes in the file content are **escaped automatically** during JSON serialization.

## v0.4.1 (2026-10-05)

### Changed
- `sys http` now **defaults to `application/json`**: every request carries `Content-Type: application/json` by default (including bodiless requests and any method), and `Accept` defaults to `application/json, */*;q=0.5`; overridable via `-f/--form`, `--multipart`, or an explicit `Content-Type: ...` header.

## v0.4.0 (2026-10-05)

### Added
- `sys ai model` / `ai cn-model --model-type` now accepts **multiple types separated by half-width `,` or full-width `，`**, AND-ed together: only models supporting **every** requested modality are kept (e.g. `--model-type text,image` shows models that handle both text and image; Chinese aliases like `文本，图像` work too).
- `sys ai chat` / `ai task` complete model capability fields **before sending each HTTP request**: when a model entry lacks `max_input_tokens` or `type`, the first models.dev match by model name is looked up — its `context` fills `max_input_tokens` and its input/output modality union fills `type` — and the values are **persisted back into the config file** (reused on later runs); lookup or match failures degrade silently.
- **Message truncation**: before sending, the message's char length is checked against the model's `max_input_tokens`; when over the limit it is cut to fit (a notice is printed to stderr), preventing context-window overflow.
- `sys ai task -t *`: **score-matching across all tasks** — each task's `desc` is combined with the user-provided request and scored by the configured LLM on a 10-point scale (0 = no match, 10 = perfect match); a `TASK / DESC / SCORE` table sorted by score is printed, and a failed task shows `-`.

### Notes
- Model entries in the config gain optional fields `max_input_tokens` (char limit) and `type` (modality union, e.g. `text,image`); they may also be pre-filled manually.
- `-t *` must be quoted in shells like PowerShell (`-t "*"`) to avoid glob expansion.

## v0.3.0 (2026-10-05)

### Added
- `sys ai chat --list-model`: lists **all model names** in the config file's `clients` section (grouped by provider, with weights); `--list-provider`: lists all providers (name / type / model count). Both may be combined and require no message.
- `sys ai chat -m/--model MODEL`: **temporarily overrides** the top-level `model`, using the exact same rules (`{provider}:{model}` / `{provider}:*` / bare `{model}` / comma-separated multi-select).
- Both the top-level `model` and `-m` accept **multiple selectors separated by half-width `,` or full-width `，`**, used in weighted rotation (e.g. `agnes:*,claude:claude-3-5-sonnet`).
- **Dynamic weighted rotation**: model `weight` now ranges 0-9 (default 1). On request failure, if `weight > 1` the value is decremented by 1 and **written back to the config file**, then the next model is tried automatically; on a fallback success, if `weight < 9` it is incremented by 1 and written back. Both the conventional `weight:` sub-key and the mangled standalone `- weight: N` form are handled, preserving indentation and trailing comments.
- `sys ai model`: text tables gained `CONTEXT` (context length) and `TYPE` (modality) columns; new `--model-type TYPE` filter: `text / image / audio / video / pdf` (Chinese aliases `文本 / 图像 / 语音 / 视频` accepted), matched against `modalities.input/output`.
- `sys ai cn-model`: list tables gained a `TYPE` (category) column; new `--model-type TYPE` filter: `text / image / audio / video / multimodal` (Chinese aliases accepted), matched against card categories (e.g. `audio` → 语音大模型, `image` → vision / multimodal). **Exact single-model hits now fetch the detail page**, outputting `context` (e.g. `1.05M`) and `modality` (e.g. `文本、图像 → 文本`), degrading silently on failure; CSV extended to 10 columns (added `context,modality`).
- Robustness: config parsing now strips a UTF-8 BOM so a leading top-level `model` key is not silently lost.

### Notes
- Existing `chat_state.json` rotation state remains compatible (a comma-containing selector is keyed as one string).

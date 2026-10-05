# sysenv Changelog

Versioned record of new features / fixes / release conventions. Artifact naming: `sysenv-<platform>-<arch>_v<version>`, release binaries compressed with UPX (see README "Build & Release").

## v0.4.5 (2026-10-05)

### Added
- `sysenv ai task` gains **10 new built-in search sources** covering the five requested data groups:
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
- `sysenv ai task` tool execution is now **pure in-code HTTP, with local shell commands banned** (the `tool:` command template was removed and `run_tool_cmd` deleted). A task can declare either of two tools which the program executes after function_call selection and feeds the **real data back** to the model:
  - **`api: <URL template>`** — fixed HTTP API: `{key}` / `{key:default}` placeholders are filled from the model's tool arguments (empty values fall back to the default), and the response body is returned as the tool result; `params: [k1, k2]` declares optional parameters, added to the function schema.
  - **`search: <source>`** — fuzzy AI web search: five built-in sources — `zhihu` (daily news), `baidu` (real-time hot search), `bilibili` (popular videos), `github` (repos created in the last 7 days, optional `date` argument), `hn` (Hacker News) — the program fetches the headline list and feeds it back for the model to filter and summarize.
- The home-grown YAML parser now supports inline arrays `[a, b]` (both `params: [lat, lon]` and block-list forms work).
- Sample config adds `topnews` (zhihu) and `topshow` (baidu) tasks; weather now uses the `api:` form.

### Fixed
- Empty tool-argument strings no longer override `{key:default}` URL placeholders (previously produced invalid URLs).

## v0.4.3 (2026-10-05)

### Added
- `sysenv ai task` without `-t` and with a user request (arguments or stdin) now uses **function_call auto-routing**:
  - Each task's `name` becomes the function name and `desc` the function description, registered as tools (both OpenAI and Anthropic protocols).
  - The LLM picks the best-matching task via function calls; the user request is carried by the `input` parameter.
  - The selected task is executed automatically: its `desc` plus the user request are assembled into the chat message and the final reply is printed.
  - When the model selects no task, an explicit error suggests using `-t NAME` instead.
- Backward compatible: no input still lists the tasks; `-t NAME` (template placeholder substitution) and `-t *` (0-10 scoring) keep their behavior; routing requests also use dynamic weighted fallback across models.

## v0.4.2 (2026-10-05)

### Added
- `sysenv http` new **`--file FILE`** flag: reads a local file as the raw request body (equivalent to the positional `@FILE`), auto-defaults to POST; mutually exclusive with the positional `@FILE` / `--raw`, with an explicit error.
- Request-item values starting with `@` (`key=@file`, `key:=@file`, `key==@file`, `key:@file`) read the local file as the field value: **UTF-8 BOM and trailing newlines are stripped automatically**, and special characters such as quotes / backslashes in the file content are **escaped automatically** during JSON serialization.

## v0.4.1 (2026-10-05)

### Changed
- `sysenv http` now **defaults to `application/json`**: every request carries `Content-Type: application/json` by default (including bodiless requests and any method), and `Accept` defaults to `application/json, */*;q=0.5`; overridable via `-f/--form`, `--multipart`, or an explicit `Content-Type: ...` header.

## v0.4.0 (2026-10-05)

### Added
- `sysenv ai model` / `ai cn-model --model-type` now accepts **multiple types separated by half-width `,` or full-width `，`**, AND-ed together: only models supporting **every** requested modality are kept (e.g. `--model-type text,image` shows models that handle both text and image; Chinese aliases like `文本，图像` work too).
- `sysenv ai chat` / `ai task` complete model capability fields **before sending each HTTP request**: when a model entry lacks `max_input_tokens` or `type`, the first models.dev match by model name is looked up — its `context` fills `max_input_tokens` and its input/output modality union fills `type` — and the values are **persisted back into the config file** (reused on later runs); lookup or match failures degrade silently.
- **Message truncation**: before sending, the message's char length is checked against the model's `max_input_tokens`; when over the limit it is cut to fit (a notice is printed to stderr), preventing context-window overflow.
- `sysenv ai task -t *`: **score-matching across all tasks** — each task's `desc` is combined with the user-provided request and scored by the configured LLM on a 10-point scale (0 = no match, 10 = perfect match); a `TASK / DESC / SCORE` table sorted by score is printed, and a failed task shows `-`.

### Notes
- Model entries in the config gain optional fields `max_input_tokens` (char limit) and `type` (modality union, e.g. `text,image`); they may also be pre-filled manually.
- `-t *` must be quoted in shells like PowerShell (`-t "*"`) to avoid glob expansion.

## v0.3.0 (2026-10-05)

### Added
- `sysenv ai chat --list-model`: lists **all model names** in the config file's `clients` section (grouped by provider, with weights); `--list-provider`: lists all providers (name / type / model count). Both may be combined and require no message.
- `sysenv ai chat -m/--model MODEL`: **temporarily overrides** the top-level `model`, using the exact same rules (`{provider}:{model}` / `{provider}:*` / bare `{model}` / comma-separated multi-select).
- Both the top-level `model` and `-m` accept **multiple selectors separated by half-width `,` or full-width `，`**, used in weighted rotation (e.g. `agnes:*,claude:claude-3-5-sonnet`).
- **Dynamic weighted rotation**: model `weight` now ranges 0-9 (default 1). On request failure, if `weight > 1` the value is decremented by 1 and **written back to the config file**, then the next model is tried automatically; on a fallback success, if `weight < 9` it is incremented by 1 and written back. Both the conventional `weight:` sub-key and the mangled standalone `- weight: N` form are handled, preserving indentation and trailing comments.
- `sysenv ai model`: text tables gained `CONTEXT` (context length) and `TYPE` (modality) columns; new `--model-type TYPE` filter: `text / image / audio / video / pdf` (Chinese aliases `文本 / 图像 / 语音 / 视频` accepted), matched against `modalities.input/output`.
- `sysenv ai cn-model`: list tables gained a `TYPE` (category) column; new `--model-type TYPE` filter: `text / image / audio / video / multimodal` (Chinese aliases accepted), matched against card categories (e.g. `audio` → 语音大模型, `image` → vision / multimodal). **Exact single-model hits now fetch the detail page**, outputting `context` (e.g. `1.05M`) and `modality` (e.g. `文本、图像 → 文本`), degrading silently on failure; CSV extended to 10 columns (added `context,modality`).
- Robustness: config parsing now strips a UTF-8 BOM so a leading top-level `model` key is not silently lost.

### Notes
- Existing `chat_state.json` rotation state remains compatible (a comma-containing selector is keyed as one string).

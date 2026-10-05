# sysenv Changelog

Versioned record of new features / fixes / release conventions. Artifact naming: `sysenv-<platform>-<arch>_v<version>`, release binaries compressed with UPX (see README "Build & Release").

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
- `sysenv ai task --list-source`: lists all built-in search sources with their **name, kind** (`api` direct HTTP API / `search` AI aggregate search with Sogou first and Bing fallback / `generic` general search engine requiring a `q`/`query` argument), **purpose and access URL** — 19 sources (zhihu/baidu/bilibili/github/hn/toutiao/tophub/oschina/smzdm/bing/sogou/dxtower/enlightent/cls/dongchedi/autohome/szhousing/penalty/company).

## v0.4.7 (2026-10-05)

### Added
- Two new search sources for `sysenv ai task`:
  - `penalty`: administrative penalty / dishonest-executor records for a person or organization (public disclosure channels, AI aggregate search).
  - `company`: company registration info (legal representative, registered capital, founding date, etc.; public disclosure channels, AI aggregate search).
- Implementation note: Creditchina (creditchina.gov.cn) is unreachable, the enterprise credit system (gsxt.gov.cn) returns 521, and the court execution site / wenshu / aiqicha all require CAPTCHA or JS rendering, so direct fetching is impossible. AI aggregate search (Sogou first, Bing fallback) over public disclosure pages (qcc / aiqicha / baike / government notices) is used instead; the entity name comes from the model's `name` argument, and the model honestly reports when no records are found.
- Manual `-t` tasks now fill the first declared parameter with a bare free-text argument (e.g. `-t company 字节跳动` → `name=字节跳动`), replacing both the `{name}` placeholder in the msg and the tool query. Existing tasks like `weather` are unaffected.
- Sample config adds the `penalty` / `company` tasks (both `~/.sysenv/config.yaml` and `doc/config.yaml`).

## v0.4.6 (2026-10-05)

### Added
- New search source `szhousing` for `sysenv ai task`: **Shenzhen housing sales** (new/second-hand transaction counts from the Shenzhen Real Estate Information Platform's public data).
- Implementation note: `fdc.zjj.sz.gov.cn` runs a Ruishi dynamic WAF (full browser headers still return HTTP 412), so it cannot be fetched directly. It uses AI aggregate search (Sogou first, Bing fallback) over public channels (Leyoujia / Centaline / the housing bureau site) and the model summarizes honestly with sources.
- Sample config adds the `szhousing` task (both `~/.sysenv/config.yaml` and `doc/config.yaml`).

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

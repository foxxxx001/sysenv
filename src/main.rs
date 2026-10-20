mod ai;
mod chat;
mod convert;
mod dasel;
mod env;
mod file;
mod hash;
mod httpie;
mod image;
mod link;
mod lock;
mod log;
mod net;
mod path;
mod short;
mod server;
mod store;
mod szfdc;
mod task;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "sys",
    version = concat!(env!("CARGO_PKG_VERSION"), " (Made by Gary-china)"),
    about = "System PATH & environment manager + httpie-compatible HTTP client (Windows / Ubuntu)",
    long_about = "sys manages the system PATH and environment variables (persisted and applied to the current environment), imports/exports PATH to the registry, links executables into a PATH directory, queries the DataLearner AI model list and the models.dev database of AI providers, chats with LLM providers configured in ~/.sysenv/config.yaml, probes domains / URLs read from a file (`sys net`, response-time / JSON / plain-text checks), installs command shims (spath/senv/slink/shttp/sai/stask/sfile), and ships an httpie-compatible HTTP client.

Examples:
  sys path list
  sys path add D:\\tools
  sys path export backup.reg
  sys env set FOO bar
  sys link myapp.exe
  sys ai model gpt-4.1
  sys ai cn-model gpt-6.1-sol
  sys ai chat \"你好\"
  sys ai image \"a red fox in the snow\"
  sys ai provider openai
  sys ai info provider
  sys ai info model
  sys ai price openai,anthropic
  sys ai price --src openrouter
  sys ai price deepseek --src openrouter
  sys ai balance minimax
  sys ai balance deepseek
  sys ai info sale-price agnes
  sys ai info list-model agnes
  sys ai info models baidu
  sys net -i urls.txt -t domain
  sys net -i urls.txt -t test -o ok.txt
  sys task list
  sys task kill 1234
  sys log --file app.log -f
  sys log --file app.log --level error --search timeout
  sys short
  sys http pie.dev/get name=John"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage PATH entries: list / add / remove / has / export / import / unlock
    Path(PathArgs),
    /// Read / set / unset / list environment variables
    Env(EnvArgs),
    /// Link a file into a PATH directory so it runs from anywhere
    Link(LinkArgs),
    /// httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
    #[command(
        disable_help_flag = true,
        about = "httpie-compatible HTTP client",
        long_about = "httpie-compatible HTTP client.

Usage: sys http [flags] [METHOD] URL [ITEM...]

Items:
  key=value      JSON/form data field
  key:=json      raw JSON value (number, bool, object, array)
  key==value     URL query parameter
  key:value      request header
  key@file       file upload (multipart)
  key=@file      embed file content as a field value
  @file          raw request body from file (stdin when piped)

Flags follow httpie: -j/--json, -f/--form, --multipart, -p/--print,
-h/--headers, -b/--body, -m/--meta, -v/--verbose, -o/--output,
-d/--download, -q/--quiet, -a/--auth, -A/--auth-type, --proxy,
-F/--follow, --max-redirects, --timeout, --check-status, --offline,
--verify, -I/--ignore-stdin, --default-scheme, --raw.

Use `sys help http` for help (in http subcommand, -h means response headers)."
    )]
    Http(HttpArgs),
    /// Query AI model info (DataLearner), provider info (models.dev), inspect the config (info), chat with configured providers
    Ai(AiArgs),
    /// Probe domains / URLs read from a file: -i FILE [-t domain|tv|file|test] [-o FILE] [--timeout SECONDS]
    Net(NetArgs),
    /// Query and kill processes (list PID/name/path; kill by PID or name; no args lists all)
    Task(TaskArgs),
    /// Colored, less-like log viewer: parse timestamps/levels/app markers with
    /// filters and search; -f tails the file (`sys log --file app.log [--level
    /// LEVEL] [--search KW] [--line N] [--newer TIME] [-f]`)
    Log(LogArgs),
    /// fd/sd-style text search and replacement: search stdin or a directory tree, or replace a string in place
    File(FileArgs),
    /// Query / modify / convert structured data (JSON / JSONL / YAML / TOML / CSV / MD / TABLE), dasel-compatible
    /// (alias `dasel`): `sys data [SELECTOR | 'PATH = VALUE'] [-f FILE] [-i FMT] [-o FMT] [-w FILE] [--compact] [--root]`
    #[command(alias = "dasel")]
    Data(DataArgs),
    /// Install shell shims for every subcommand (spath/senv/slink/shttp/sai/stask/sfile)
    Short(ShortArgs),
}

#[derive(Args)]
struct DataArgs {
    /// Selector (`a.b[0]`, `list[*]`, `m["中文键"]`) or assignment expression
    /// (`PATH = VALUE`, `PATH += N`, `PATH = null` deletes); omitted = pure
    /// format conversion
    #[arg(value_name = "EXPR")]
    expr: Option<String>,
    /// Read the input from FILE instead of stdin (the format comes from the
    /// file extension unless -i overrides it)
    #[arg(short = 'f', long, value_name = "FILE", conflicts_with = "url")]
    file: Option<PathBuf>,
    /// GET this URL and use the response body as the input (default format
    /// json; -i overrides). Example: `sys data -u https://api.dev/config status`
    #[arg(short = 'u', long, value_name = "URL", conflicts_with = "file")]
    url: Option<String>,
    /// Input format: json | jsonl | yaml | toml | csv | md | table (default: -f extension, else detection)
    #[arg(short = 'i', long, value_name = "FMT")]
    input: Option<String>,
    /// Output format (default: the input format)
    #[arg(short = 'o', long, value_name = "FMT")]
    output: Option<String>,
    /// `-i csv` / `-i table` only: treat the first non-blank line as the
    /// column header. The first line is the header by default; a second line
    /// made of `----` separators is detected and skipped. Use `--no-header`
    /// to get col1..colN instead
    #[arg(long, conflicts_with = "no_header")]
    header: bool,
    /// `-i csv` / `-i table` only: treat every line as data and name the
    /// columns col1..colN (required for colN keys; never auto-inferred)
    #[arg(long, conflicts_with = "header")]
    no_header: bool,
    /// Write the result back to FILE instead of stdout (with an expression the
    /// whole modified document is written; also pass --root to keep stdout quiet)
    #[arg(short = 'w', long, value_name = "FILE")]
    write: Option<PathBuf>,
    /// Compact output (single-line JSON)
    #[arg(long)]
    compact: bool,
    /// Print the whole document after a modification instead of just the value
    #[arg(long)]
    root: bool,
    /// Delete every node deeper than N levels from the whole document (root =
    /// level 0; levels 0..=N are kept). Bare `--depth` means 1; applied before
    /// the expression.
    #[arg(long, value_name = "N", num_args = 0..=1, default_missing_value = "1")]
    depth: Option<usize>,
    /// Keep only the first N rows when the document is an array (csv / table
    /// input parses to an array of rows; jsonl parses to an array of JSON
    /// values). Applied to the whole document before the expression, so
    /// `sys data -i table --limit 3` prints the first 3 rows (header still
    /// included).
    #[arg(long, value_name = "N")]
    limit: Option<usize>,
}

#[derive(Args)]
struct PathArgs {
    #[command(subcommand)]
    cmd: PathCmd,
}

#[derive(Subcommand)]
enum PathCmd {
    /// List the effective PATH entries of the current process
    List,
    /// Add a directory to PATH (current session only by default; --save persists for new processes)
    Add {
        /// Directory to add (relative paths are resolved to absolute)
        dir: String,
        /// Prepend instead of append
        #[arg(short = 'p', long)]
        prepend: bool,
        /// Persist the change so new processes inherit it (default: current session only)
        #[arg(long)]
        save: bool,
        /// Scope of the persisted change: user (default) or machine (admin)
        #[arg(long, value_enum, default_value_t = store::Scope::User)]
        scope: store::Scope,
    },
    /// Remove a directory from PATH
    Remove {
        /// Directory to remove
        dir: String,
        /// Do not persist; print a snippet to apply to the current shell only
        #[arg(long)]
        temporary: bool,
        /// Scope of the change: user (default) or machine (admin)
        #[arg(long, value_enum, default_value_t = store::Scope::User)]
        scope: store::Scope,
    },
    /// Check whether a directory is on the PATH
    Has {
        /// Directory to look up
        dir: String,
    },
    /// Export the effective PATH to a file (.reg / .json / .txt)
    Export {
        /// Output file (extension selects the format)
        file: String,
    },
    /// Import PATH entries from a file into the persisted PATH
    Import {
        /// Input file (.reg / .json / .txt)
        file: String,
        /// Replace the persisted PATH instead of merging
        #[arg(long)]
        replace: bool,
        /// Scope of the change: user (default) or machine (admin)
        #[arg(long, value_enum, default_value_t = store::Scope::User)]
        scope: store::Scope,
    },
    /// Check whether a file/directory is locked by other processes; list the
    /// lockers (PID / name / path) and unlock it by shutting them down (Windows)
    Unlock {
        /// File or directory to check / unlock (relative paths resolve against the current directory)
        path: String,
    },
}

#[derive(Args)]
struct EnvArgs {
    #[command(subcommand)]
    cmd: EnvCmd,
}

#[derive(Subcommand)]
enum EnvCmd {
    /// Read an environment variable (process value first, then persisted)
    Get {
        /// Variable name
        name: String,
    },
    /// Set an environment variable (persisted by default; --temporary prints a snippet)
    Set {
        /// Variable name
        name: String,
        /// Value
        value: String,
        /// Do not persist; print a snippet to apply to the current shell only
        #[arg(long)]
        temporary: bool,
        /// Scope of the change: user (default) or machine (admin)
        #[arg(long, value_enum, default_value_t = store::Scope::User)]
        scope: store::Scope,
    },
    /// Remove an environment variable
    Unset {
        /// Variable name
        name: String,
        /// Do not persist; print a snippet to apply to the current shell only
        #[arg(long)]
        temporary: bool,
        /// Scope of the change: user (default) or machine (admin)
        #[arg(long, value_enum, default_value_t = store::Scope::User)]
        scope: store::Scope,
    },
    /// List all environment variables of the current process
    List,
}

#[derive(Args)]
struct LinkArgs {
    /// File to link (relative paths are resolved against the current directory)
    file: PathBuf,
    /// Place the link in this directory instead of the managed bin directory
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
    /// Use the system directory (C:\\Windows\\System32 / /usr/local/bin)
    #[arg(long)]
    system: bool,
    /// Link method: hard (default, falls back to symlink then copy), symlink, copy
    #[arg(long, value_enum)]
    method: Option<link::LinkMethod>,
    /// Override the linked command name
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Overwrite an existing link
    #[arg(short = 'f', long)]
    force: bool,
    /// Do not persist the PATH addition; print a snippet instead
    #[arg(long)]
    temporary: bool,
    /// Windows only: do not create a .cmd shim for non-.exe targets
    #[arg(long)]
    no_shim: bool,
}

#[derive(Args)]
struct AiArgs {
    /// Force re-fetching the remote data (otherwise use the 24 h cache)
    #[arg(long)]
    refresh: bool,
    #[command(subcommand)]
    cmd: AiCmd,
}

#[derive(Subcommand)]
enum AiCmd {
    /// Query models.dev for model information by name (no args: list every model)
    Model(AiModelArgs),
    /// Query the DataLearner AI model list by name (published date filter via --date)
    CnModel(CnModelArgs),
    /// Query models.dev for provider information by name
    Provider(AiProviderArgs),
    /// Inspect the local config: providers (name/api_base/api_key), models ({provider}:{name}), model prices (models.dev)
    Info(AiInfoArgs),
    /// Query a provider's official balance with its api_key (minimax / agnes / deepseek have API handlers; the rest point to their consoles)
    Balance(AiBalanceArgs),
    /// Show model prices (models.dev by default; --src openrouter for openrouter.ai pricing)
    Price(AiPriceArgs),
    /// Chat with an LLM configured in ~/.sysenv/config.yaml (OpenAI / Anthropic compatible)
    Chat(ChatArgs),
    /// Serve the configured providers as an OpenAI-compatible API server
    /// (default 127.0.0.1:10000; a bare port or a bare IP keeps the other default)
    Server(ServerArgs),
    /// Generate images via an OpenAI-compatible Images API (/v1/images/generations)
    Image(ImageArgs),
    /// Export the local config to another tool's format (codex | opencode | litellm | freellmapi | codebuddy | workbuddy | trae | zcoder);
    /// --show previews the target config file path and content, -f writes with a backup,
    /// --update merges into the software's default config file (backup first)
    Config(AiConfigArgs),
}

#[derive(Args)]
struct AiConfigArgs {
    /// Target format: codex (TOML) | opencode (JSON) | litellm (YAML) | freellmapi (JSON) |
    /// codebuddy | workbuddy (models.json) | trae (trae_cli.yaml) | zcoder (ZCode config.json);
    /// omitted when --list is given
    #[arg(value_name = "FORMAT", required_unless_present = "list")]
    format: Option<String>,
    /// Select models: omit for the same default as `sys ai chat` (top-level `model:`);
    /// `provider:*` = a provider's whole model list; `provider:model` = one specific model
    /// (legacy single selector; prefer -m)
    #[arg(value_name = "SELECT")]
    provider: Option<String>,
    /// Select models to export, same comma-separated forms as `sys ai chat -m`
    /// ({provider}:{model} / {provider}:* / bare model name, half/full-width commas)
    #[arg(short = 'm', long, value_name = "MODEL", conflicts_with = "provider")]
    models: Option<String>,
    /// Write the export to this file instead of stdout (the previous file is backed up first)
    #[arg(short = 'f', long, value_name = "FILE")]
    file: Option<PathBuf>,
    /// Agent config directory: the target software's usual config file name is
    /// looked up directly inside this dir, replacing the default home-relative
    /// path for --show / --update / --import / --list (for agents installed at
    /// a non-default place). -f FILE always wins
    #[arg(long, value_name = "DIR")]
    src: Option<PathBuf>,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Show mode: print the target software's config file path, then the config
    /// that would be applied (no file is written)
    #[arg(long, conflicts_with = "list")]
    show: bool,
    /// Update the software's default config file in place: back up a .bak file first,
    /// merge the missing selected models into it, and report what was added
    /// (codebuddy / workbuddy / trae / zcoder; -f FILE overrides the default path)
    #[arg(long, conflicts_with_all = ["show", "list"])]
    update: bool,
    /// Import mode: read provider/model params from the software's config file
    /// (or -f FILE as the source) and merge them into ~/.sysenv/config.yaml
    /// (backed up first; providers/models already present are kept).
    /// Only codebuddy / workbuddy / zcoder can be imported
    #[arg(long, conflicts_with_all = ["show", "list", "update"])]
    import: bool,
    /// List every supported format and its default config file path
    #[arg(long)]
    list: bool,
}

#[derive(Args)]
struct AiModelArgs {
    /// Model name or id to look up (e.g. gpt-4.1 or openai/gpt-4.1); omit to list every model
    name: Option<String>,
    /// Substring search over model ids and names
    #[arg(short = 's', long, value_name = "QUERY", conflicts_with = "name")]
    search: Option<String>,
    /// List available models
    #[arg(long, conflicts_with_all = ["name", "search"])]
    list: bool,
    /// Maximum number of entries to show when listing (default 20 with --list; all when no arguments)
    #[arg(long)]
    limit: Option<usize>,
    /// Show only models whose last_updated is after DATE (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    date: Option<String>,
    /// Show only models supporting this modality: text | image | audio | video | pdf (or 文本/图像/语音/视频)
    #[arg(long, value_name = "TYPE")]
    model_type: Option<String>,
    /// Show only models of providers whose id or name contains this value (case-insensitive substring)
    #[arg(long, value_name = "PROVIDER")]
    provider: Option<String>,
    /// Show only models whose cost.input and cost.output are both <= PRICE (default 0 = free only; missing cost counts as 0)
    #[arg(long, value_name = "PRICE")]
    price: Option<f64>,
    /// Show only models with open_weights enabled
    #[arg(long)]
    open: bool,
    /// Print the raw JSON (as a JSON array) instead of the formatted view
    #[arg(long)]
    json: bool,
    /// Output format: json (JSON array) or csv (table); default is a formatted text view
    #[arg(short = 'o', long = "output-format", value_name = "FORMAT", value_enum, conflicts_with = "json")]
    output: Option<OutFormat>,
    /// Data source: models (default, models.dev) | openrouter (openrouter.ai catalogue)
    #[arg(long = "src", value_name = "SRC", default_value = "models")]
    src: String,
}

#[derive(Args)]
struct AiPriceArgs {
    /// Comma-separated provider names to look up models.dev prices (src=models);
    /// with --src openrouter: optional model filter (comma-separated model id/name terms)
    #[arg(value_name = "P1,P2,...")]
    providers: Vec<String>,
    /// Data source: models (default, models.dev) | openrouter (openrouter.ai pricing)
    #[arg(long = "src", value_name = "SRC", default_value = "models")]
    src: String,
    /// Maximum number of entries to show (src=openrouter)
    #[arg(long)]
    limit: Option<usize>,
    /// Print the raw JSON (as a JSON array) instead of the formatted view (src=openrouter)
    #[arg(long)]
    json: bool,
    /// Output format: json / csv / yaml / md (src=openrouter)
    #[arg(short = 'o', long = "output-format", value_name = "FORMAT", value_enum, conflicts_with = "json")]
    output: Option<OutFormat>,
}

#[derive(Args)]
struct CnModelArgs {
    /// Model name / alias / id (slug) to look up (e.g. "GPT-6.1 Sol" or gpt-6-1-sol)
    name: Option<String>,
    /// Substring search over model names, ids and aliases
    #[arg(short = 's', long, value_name = "QUERY", conflicts_with = "name")]
    search: Option<String>,
    /// List every model (full catalogue)
    #[arg(long, conflicts_with_all = ["name", "search"])]
    list: bool,
    /// Maximum number of entries to show for --search results (default 20; --list shows all)
    #[arg(long)]
    limit: Option<usize>,
    /// Show only models whose published date is after DATE (YYYY-MM-DD)
    #[arg(long, value_name = "DATE")]
    date: Option<String>,
    /// Show only models of this kind: text | image | audio | video | multimodal (or 文本/图像/语音/视频/多模态)
    #[arg(long, value_name = "TYPE")]
    model_type: Option<String>,
    /// Print the raw JSON (as a JSON array) instead of the formatted view
    #[arg(long)]
    json: bool,
    /// Output format: json (JSON array) or csv (table); default is a formatted text view
    #[arg(short = 'o', long = "output-format", value_name = "FORMAT", value_enum, conflicts_with = "json")]
    output: Option<OutFormat>,
}

#[derive(Args)]
struct AiInfoArgs {
    /// What to show: provider | model | sale-price | list-model | models (omitted: the configured providers and models together; prices moved to `sys ai price`)
    #[arg(value_name = "FIELD")]
    field: Option<String>,
    /// provider KEYWORD: keep only providers whose name contains KEYWORD;
    /// model KEYWORD: keep only models of providers whose name contains KEYWORD
    /// (or `provider:model` / `provider:*` to select specific models);
    /// sale-price PROVIDER: scrape the provider's official pricing page;
    /// list-model / models PROVIDER: query the provider's own models API and
    /// list the models its configured api_key can use (same as
    /// `sys ai chat --query-model PROVIDER`)
    #[arg(value_name = "PARAM")]
    param: Vec<String>,
    /// Force re-fetching the models.dev price data (otherwise use the 24 h cache)
    #[arg(long)]
    refresh: bool,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Print the raw JSON (as a JSON array) instead of the formatted view
    /// (supported by `info provider`, `info model` and `info list-model`)
    #[arg(long)]
    json: bool,
    /// Output format: json (JSON array), csv (table), yaml (list) or md (markdown table); default is a formatted
    /// text view (supported by `info provider`, `info model` and `info list-model`)
    #[arg(short = 'o', long = "output-format", value_name = "FORMAT", value_enum, conflicts_with = "json")]
    output: Option<OutFormat>,
}

#[derive(Args)]
struct AiBalanceArgs {
    /// Query the provider's official balance with its api_key:
    /// minimax / agnes / deepseek have API handlers, the rest point to their
    /// consoles (moved up from `sys ai info balance`)
    #[arg(value_name = "PROVIDER")]
    provider: String,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[derive(Args)]
struct ChatArgs {
    /// The message to send (multiple words are joined with spaces; when omitted, stdin is read when piped)
    #[arg(value_name = "MSG")]
    msg: Vec<String>,
    /// Override the top-level model from the config; accepts {provider}:{model}, {model} or comma-separated (half/full-width) model lists
    #[arg(short = 'm', long, value_name = "MODEL")]
    model: Option<String>,
    /// List every model configured under `clients` (grouped by provider) and exit
    #[arg(long)]
    list_model: bool,
    /// List every provider configured under `clients` and exit
    #[arg(long)]
    list_provider: bool,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Print the actual HTTP request (method/URL/headers/body) and response (status/headers/body)
    #[arg(long)]
    debug: bool,
    /// Disable streaming even if the config sets stream: true
    #[arg(long)]
    no_stream: bool,
    /// Fill missing max_input_tokens / type of the target models into the config
    /// file (looked up in models.dev, 24 h cache; best effort — only runs with this flag)
    #[arg(long)]
    update: bool,
    /// Test chat feedback time: send a streaming chat to the selected models
    /// (default: every configured model), measure only the time until the first
    /// token arrives, and set `weight: 0` in the config for models that fail
    /// (HTTP error such as 429, connection error, timeout, no content)
    #[arg(long)]
    test: bool,
    /// Number of models to test in parallel (default: the CPU core count)
    #[arg(long, value_name = "N")]
    parallel: Option<usize>,
    /// With --test: delete the failed models from the config file instead of
    /// setting weight to 0 (the original config is backed up first); providers
    /// left without any models are removed too
    #[arg(long, requires = "test")]
    remove: bool,
    /// Query a configured provider's model list through its models endpoint and
    /// exit (`{api_base}/models` for OpenAI-compatible — `models` is appended
    /// to the configured api_base as-is, e.g. Baidu serves it at
    /// `https://qianfan.baidubce.com/v2/models` — and `{api_base}/v1/models`
    /// for Anthropic; PROVIDER = a `name` under `clients`, e.g. baidu)
    #[arg(long, value_name = "PROVIDER")]
    query_model: Option<String>,
}

/// `sys ai server [ADDR]` — serve the configured providers as an
/// OpenAI-compatible API (see `sys ai chat --help` for chat options).
#[derive(Args)]
#[command(disable_help_flag = true)]
struct ServerArgs {
    /// Listen address: empty -> 127.0.0.1:10000; a bare port or a bare IP keeps the other default
    #[arg(value_name = "ADDR")]
    addr: Option<String>,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Query whether a server is already running at ADDR: prints the address/port
    /// plus the configured providers & models (running or not)
    #[arg(short = 'h', long)]
    help: bool,
}

#[derive(Args)]
struct ImageArgs {
    /// The prompt describing the image (multiple words are joined with spaces; when omitted, stdin is read when piped)
    #[arg(value_name = "PROMPT")]
    prompt: Vec<String>,
    /// Override the top-level model from the config; accepts {provider}:{model}, {model} or comma-separated (half/full-width) model lists
    #[arg(short = 'm', long, value_name = "MODEL")]
    model: Option<String>,
    /// List every model configured under `clients` (grouped by provider) and exit
    #[arg(long)]
    list_model: bool,
    /// List every provider configured under `clients` and exit
    #[arg(long)]
    list_provider: bool,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Print the actual HTTP request (method/URL/headers/body) and response (status/headers/body)
    #[arg(long)]
    debug: bool,
    /// Save the generated images into this directory (default: current directory)
    #[arg(short = 'o', long, value_name = "DIR")]
    output: Option<PathBuf>,
    /// Number of images to generate (default 1)
    #[arg(short = 'n', long, value_name = "N", default_value_t = 1)]
    count: u32,
    /// Image size passed to the API (default 1024x1024; e.g. 512x512, 1024x1024, 1792x1024)
    #[arg(short = 's', long, value_name = "SIZE", default_value = image::DEFAULT_SIZE)]
    size: String,
    /// Request image URLs instead of base64 data (the URLs are downloaded and saved)
    #[arg(long)]
    url: bool,
}

#[derive(Args)]
struct AiProviderArgs {
    /// Provider id or name to look up (e.g. openai)
    name: Option<String>,
    /// Substring search over provider ids and names
    #[arg(short = 's', long, value_name = "QUERY", conflicts_with = "name")]
    search: Option<String>,
    /// List available providers
    #[arg(long, conflicts_with_all = ["name", "search"])]
    list: bool,
    /// Maximum number of entries to show when listing (default 20)
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Print the raw JSON (as a JSON array) instead of the formatted view
    #[arg(long)]
    json: bool,
    /// Output format: json (JSON array) or csv (table); default is a formatted text view
    #[arg(short = 'o', long = "output-format", value_name = "FORMAT", value_enum, conflicts_with = "json")]
    output: Option<OutFormat>,
}

/// Machine-readable output formats for `sys ai`.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    /// JSON array output
    Json,
    /// CSV table output
    Csv,
    /// YAML list output
    Yaml,
    /// Markdown table output
    Md,
}

#[derive(Args)]
struct TaskArgs {
    /// Show only the process using PORT (list mode, e.g. `sys task -o 8080`)
    #[arg(short = 'o', long, value_name = "PORT")]
    port: Option<u16>,
    /// Subcommand; omitted to list all processes
    #[command(subcommand)]
    cmd: Option<TaskCmd>,
}

#[derive(Subcommand)]
enum TaskCmd {
    /// List processes (PID, name, path); NAME fuzzy-matches the process name
    List {
        /// Fuzzy match on the process name; a numeric value matches the PID
        name: Option<String>,
        /// Show only the process using PORT (e.g. -o 8080)
        #[arg(short = 'o', long, value_name = "PORT")]
        port: Option<u16>,
    },
    /// Kill a process by PID or by name (name is fuzzy-matched; every match is killed)
    Kill {
        /// PID or process name
        target: String,
        /// Force the kill (SIGKILL on Linux)
        #[arg(short = 'f', long)]
        force: bool,
    },
}

#[derive(Args)]
struct NetArgs {
    /// Input file containing http(s) addresses (lines starting with # are skipped; each line may hold several addresses); not used in -t info mode
    #[arg(short = 'i', long, value_name = "FILE")]
    input: Option<PathBuf>,
    /// Detection type: domain (default, 3 probes averaged) | tv (body must be JSON, analyzed with --sort; old name json) | file (body must be plain text) | test (probe reachability, delete unreachable lines, -o writes the cleaned source) | info (query domain registration info via RDAP for each -n DOMAIN)
    #[arg(short = 't', long, value_name = "TYPE", value_enum, default_value_t = net::NetType::Domain)]
    kind: net::NetType,
    /// Write the passed (reachable + format-matched) URLs, or the cleaned source lines in test mode, to this file
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Per-request timeout in seconds (default 10)
    #[arg(long, value_name = "SECONDS", default_value_t = net::DEFAULT_TIMEOUT_SECS)]
    timeout: f64,
    /// In tv/tvs mode, how many addresses are probed concurrently (default: number of CPU cores)
    #[arg(long = "thread", value_name = "NUM")]
    threads: Option<usize>,
    /// In tv/tvs mode, the keyword counted in `.sites.name` for the `name` statistic (default 4k; the output label shows this keyword, e.g. `4k=17`)
    #[arg(long, value_name = "WORD", default_value = "4k")]
    words: String,
    /// In test mode, skip probing URLs whose file-name suffix matches (case-insensitive, leading dot optional) and remove them directly; repeatable, e.g. -x m3u -x zip
    #[arg(short = 'x', long, value_name = "SUFFIX")]
    exclude: Vec<String>,
    /// tv mode sort keys, applied in order (count | name | speed); default: count name; repeatable, e.g. --sort count name
    #[arg(long, value_name = "KEY", num_args = 1..)]
    sort: Vec<String>,
    /// Domain names whose registration info is queried in -t info mode (repeatable, e.g. -n a.com -n b.com); when a name has no registration data its parent domain is tried
    #[arg(short = 'n', long, value_name = "DOMAIN")]
    names: Vec<String>,
}

#[derive(Args)]
struct FileArgs {
    /// PATTERN, or PATTERN PATH, or OLD NEW PATH (path = file or directory);
    /// or -md5/-sha FILE [EXPECTED] to hash a file; or --enc SRC [DST]
    /// [--from ENC] [--to ENC] to convert between utf-8 / gbk / gb2312:
    /// --from ENC (alias -f ENC) fixes the source encoding (omit to
    /// auto-detect), --to ENC sets the target (default utf-8); ENC =
    /// utf-8 | gbk | gb2312; --enc aliases --encode / --convert. These
    /// flag-like tokens are absorbed into ARGS, like `data`'s raw tokens, so
    /// they may appear at any position
    #[arg(value_name = "ARGS", allow_hyphen_values = true)]
    args: Vec<String>,
    /// Only search files with this extension (repeatable; leading dot optional, e.g. -e py -e md)
    #[arg(short = 'e', long, value_name = "EXT")]
    ext: Vec<String>,
    /// Case-insensitive matching
    #[arg(short = 'i', long)]
    ignore_case: bool,
    /// Only search plain-text files (txt/md/log/...); by default source-code files are searched too
    #[arg(short = 't', long)]
    text_only: bool,
    /// Match whole words only
    #[arg(short = 'w', long)]
    word: bool,
    /// Show NUM lines of context around every match
    #[arg(short = 'c', long, value_name = "NUM")]
    context: Option<usize>,
    /// Only files at least SIZE bytes (plain number = bytes; 2k/2m/2g/2t = 1024-based)
    #[arg(short = 'S', long, value_name = "SIZE")]
    size: Option<String>,
    /// Only files modified at or after TIME (YYYY-MM-DD [HH:MM[:SS]])
    #[arg(long, value_name = "TIME")]
    newer: Option<String>,
    /// Only files modified before TIME
    #[arg(long, value_name = "TIME")]
    older: Option<String>,
    /// Only descend NUM levels of subdirectories (0 = current dir only)
    #[arg(short = 'd', long, value_name = "NUM")]
    max_depth: Option<usize>,
    /// Count lines instead of printing them: -l FILE counts the file's lines,
    /// -l with no argument counts stdin, -l PATTERN [PATH] counts matching lines.
    /// May appear in any position (a trailing -l is re-scanned from ARGS, like
    /// the -md5/-sha markers), so `sys file PATTERN -l` also works
    #[arg(short = 'l', long)]
    line_count: bool,
    /// Prepend the line number to every printed line (stdin search; directory
    /// search output already contains the line number). May appear in any
    /// position (a trailing -n is re-scanned from ARGS, like -l / -md5 / -sha)
    #[arg(short = 'n', long)]
    line_numbers: bool,
}

#[derive(Args)]
struct LogArgs {
    /// Log file to parse (default: stdin when piped; `-` also means stdin)
    #[arg(long, value_name = "FILE")]
    file: Option<PathBuf>,
    /// Filter by log level: trace | debug | info | warn | warning | error | fatal
    /// (warn == warning, err == error, critical/crit/alert/emerg == fatal)
    #[arg(long, value_name = "LEVEL")]
    level: Option<String>,
    /// Search keyword (case-insensitive); line numbers are shown and n/u jump to
    /// the next/previous match
    #[arg(long, value_name = "KEYWORD")]
    search: Option<String>,
    /// Parse from this original line number onwards (1-based)
    #[arg(long, value_name = "N")]
    line: Option<usize>,
    /// Parse only lines whose timestamp is at or after TIME (any supported log
    /// time format, or YYYY-MM-DD [HH:MM[:SS]], default midnight)
    #[arg(long, value_name = "TIME")]
    newer: Option<String>,
    /// Follow the file: keep scrolling as new lines are appended (tail -f style)
    #[arg(short = 'f', long)]
    follow: bool,
}

#[derive(Args)]
struct ShortArgs {
    /// Install the shims into this directory instead of the managed bin directory
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
    /// Overwrite existing shims
    #[arg(short = 'f', long)]
    force: bool,
    /// Do not persist the PATH addition; print a snippet instead
    #[arg(long)]
    temporary: bool,
}

#[derive(Args)]
struct HttpArgs {
    /// Print parameter descriptions and examples, then exit (use `sys help http` for clap help)
    #[arg(long)]
    help: bool,
    /// Print the actual HTTP request (method/URL/headers/body) and response (status/headers/body) to stderr
    #[arg(long)]
    debug: bool,
    /// Serialize data items as a JSON object (default when data items exist)
    #[arg(short = 'j', long)]
    json: bool,
    /// Serialize data items as application/x-www-form-urlencoded
    #[arg(short = 'f', long, conflicts_with = "json")]
    form: bool,
    /// Always send multipart/form-data
    #[arg(long, conflicts_with_all = ["json", "form"])]
    multipart: bool,
    /// Raw request body without extra processing
    #[arg(long, value_name = "DATA")]
    raw: Option<String>,
    /// Read FILE as the raw request body (like the positional @FILE; cannot be combined with @FILE/stdin)
    #[arg(long, value_name = "FILE", conflicts_with = "raw")]
    file: Option<PathBuf>,
    /// What to print: H (request headers) B (request body) h (response headers) b (response body) m (response metadata)
    #[arg(short = 'p', long, value_name = "WHAT")]
    print: Option<String>,
    /// Print only the response headers
    #[arg(short = 'h', long, conflicts_with = "body")]
    headers: bool,
    /// Print only the response body
    #[arg(short = 'b', long, conflicts_with = "headers")]
    body: bool,
    /// Print only the response metadata (status line)
    #[arg(short = 'm', long, conflicts_with_all = ["headers", "body"])]
    meta: bool,
    /// Print the whole HTTP exchange (request and response)
    #[arg(short = 'v', long)]
    verbose: bool,
    /// Save the response body to FILE instead of stdout
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// wget-style download: save the body, guess the filename
    #[arg(short = 'd', long)]
    download: bool,
    /// Do not print to stdout/stderr (except errors)
    #[arg(short = 'q', long)]
    quiet: bool,
    /// Output formatting: none | all | colors | format (default: none when stdout is redirected)
    #[arg(long, value_name = "MODE")]
    pretty: Option<String>,
    /// Authentication credentials: USER[:PASS] (basic) or TOKEN (bearer)
    #[arg(short = 'a', long, value_name = "USER[:PASS]|TOKEN")]
    auth: Option<String>,
    /// Authentication type: basic (default) or bearer
    #[arg(short = 'A', long, value_name = "TYPE")]
    auth_type: Option<String>,
    /// Proxy per protocol: http:URL, https:URL or all:URL (repeatable)
    #[arg(long, value_name = "PROTOCOL:URL")]
    proxy: Vec<String>,
    /// Follow 30x redirects
    #[arg(short = 'F', long)]
    follow: bool,
    /// Maximum number of redirects (with --follow; default 30)
    #[arg(long, value_name = "N")]
    max_redirects: Option<u32>,
    /// Connection timeout in seconds (0 = no timeout)
    #[arg(long, value_name = "SECONDS")]
    timeout: Option<f64>,
    /// Exit with 3/4/5 when the status code is 3xx/4xx/5xx
    #[arg(long)]
    check_status: bool,
    /// Build and print the request without sending it
    #[arg(long)]
    offline: bool,
    /// SSL verification: yes (default), no, or a CA bundle file path
    #[arg(long, value_name = "MODE")]
    verify: Option<String>,
    /// Do not read stdin
    #[arg(short = 'I', long)]
    ignore_stdin: bool,
    /// Default URL scheme when omitted (default: http)
    #[arg(long, value_name = "SCHEME")]
    default_scheme: Option<String>,
    /// URL and request items (place item names starting with '-' after `--`)
    #[arg(value_name = "ARGS")]
    args: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result: anyhow::Result<i32> = match cli.cmd {
        Cmd::Path(p) => run_path(p).map(|_| 0),
        Cmd::Env(e) => run_env(e).map(|_| 0),
        Cmd::Link(l) => run_link(l).map(|_| 0),
        Cmd::Http(h) => run_http(h),
        Cmd::Ai(a) => run_ai(a).map(|_| 0),
        Cmd::Net(n) => run_net(n).map(|_| 0),
        Cmd::File(f) => file::cmd_file(
            &f.args,
            &f.ext,
            f.ignore_case,
            f.text_only,
            f.word,
            f.context,
            f.size,
            f.newer,
            f.older,
            f.max_depth,
            f.line_count,
            f.line_numbers,
        )
        .map(|_| 0),
        Cmd::Data(d) => dasel::cmd_data(
            d.expr.as_deref(),
            d.file.as_deref().map(|p| p.to_str().unwrap_or_default()),
            d.url.as_deref(),
            d.input.as_deref(),
            d.output.as_deref(),
            d.write.as_deref().map(|p| p.to_str().unwrap_or_default()),
            d.compact,
            d.root,
            d.depth,
            match (d.header, d.no_header) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            },
            d.limit,
        )
        .map(|_| 0),
        Cmd::Task(t) => run_task(t).map(|_| 0),
        Cmd::Short(s) => run_short(s).map(|_| 0),
        Cmd::Log(l) => log::cmd_log(
            l.file.as_deref(),
            l.level.as_deref(),
            l.search.as_deref(),
            l.line,
            l.newer.as_deref(),
            l.follow,
        )
        .map(|_| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("sys: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_path(p: PathArgs) -> anyhow::Result<()> {
    match p.cmd {
        PathCmd::List => path::cmd_list(),
        PathCmd::Add { dir, prepend, save, scope } => {
            path::cmd_add(&dir, scope, prepend, save)
        }
        PathCmd::Remove { dir, temporary, scope } => path::cmd_remove(&dir, scope, temporary),
        PathCmd::Has { dir } => path::cmd_has(&dir),
        PathCmd::Export { file } => path::cmd_export(&file),
        PathCmd::Import { file, replace, scope } => path::cmd_import(&file, scope, replace),
        PathCmd::Unlock { path } => lock::cmd_path_unlock(&path),
    }
}

fn run_env(e: EnvArgs) -> anyhow::Result<()> {
    match e.cmd {
        EnvCmd::Get { name } => env::cmd_get(&name, store::Scope::User),
        EnvCmd::Set { name, value, temporary, scope } => {
            env::cmd_set(&name, &value, scope, temporary)
        }
        EnvCmd::Unset { name, temporary, scope } => env::cmd_unset(&name, scope, temporary),
        EnvCmd::List => env::cmd_list(),
    }
}

fn run_link(l: LinkArgs) -> anyhow::Result<()> {
    let file = l.file.to_string_lossy().into_owned();
    link::cmd_link(
        &file,
        l.dir,
        l.system,
        l.method,
        l.name,
        l.force,
        l.temporary,
        l.no_shim,
    )
}

fn run_ai(a: AiArgs) -> anyhow::Result<()> {
    match a.cmd {
        AiCmd::Model(m) => ai::cmd_model(
            m.name.as_deref(),
            m.search.as_deref(),
            m.list,
            m.limit,
            m.json,
            m.output,
            a.refresh,
            m.date.as_deref(),
            m.open,
            m.model_type.as_deref(),
            m.provider.as_deref(),
            m.price,
            Some(m.src.as_str()),
            m.limit,
        ),
        AiCmd::CnModel(m) => ai::cmd_cn_model(
            m.name.as_deref(),
            m.search.as_deref(),
            m.list,
            m.limit,
            m.json,
            m.output,
            a.refresh,
            m.date.as_deref(),
            m.model_type.as_deref(),
        ),
        AiCmd::Provider(p) => ai::cmd_provider(
            p.name.as_deref(),
            p.search.as_deref(),
            p.list,
            p.limit,
            p.json,
            p.output,
            a.refresh,
        ),
        AiCmd::Info(i) => ai::cmd_info(i.field.as_deref(), &i.param, i.refresh, i.config.as_deref(), i.output, i.json),
        AiCmd::Balance(b) => ai::cmd_balance(&b.provider, b.config.as_deref()),
        AiCmd::Price(p) => ai::cmd_price(
            &p.providers,
            Some(p.src.as_str()),
            p.limit,
            p.json,
            p.output,
            a.refresh,
        ),
        AiCmd::Chat(c) => chat::cmd_chat(
            &c.msg,
            c.config.as_deref(),
            c.debug,
            c.no_stream,
            c.model.as_deref(),
            c.list_model,
            c.list_provider,
            c.update,
            c.test,
            c.parallel,
            c.remove,
            c.query_model.as_deref(),
        ),
        AiCmd::Server(s) => {
            let bind = server::parse_addr(s.addr.as_deref().unwrap_or(""));
            if s.help {
                if server::is_running(&bind) {
                    println!(
                        "sys: server is RUNNING at http://{bind} (OpenAI-compatible: POST /v1/chat/completions, GET /v1/models)"
                    );
                } else {
                    println!(
                        "sys: server is NOT running at http://{bind} — start it with `sys ai server [ADDR]`"
                    );
                }
                ai::print_server_config(s.config.as_deref())
            } else {
                server::serve(s.config.as_deref(), &bind)
            }
        }
        AiCmd::Image(i) => image::cmd_image(
            &i.prompt,
            i.config.as_deref(),
            i.debug,
            i.model.as_deref(),
            i.list_model,
            i.list_provider,
            i.output,
            i.count,
            &i.size,
            i.url,
        ),
        AiCmd::Config(g) => ai::cmd_config(
            g.format.as_deref(),
            g.provider.as_deref(),
            g.file.as_deref(),
            g.config.as_deref(),
            g.show,
            g.models.as_deref(),
            g.update,
            g.list,
            g.import,
            g.src.as_deref(),
        ),
    }
}

fn run_short(s: ShortArgs) -> anyhow::Result<()> {
    short::cmd_short(s.dir, s.force, s.temporary)
}

fn run_net(n: NetArgs) -> anyhow::Result<()> {
    if n.kind == net::NetType::Info {
        return net::cmd_net_info(&n.names, n.timeout);
    }
    let input = n.input.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "-i/--input is required for -t {:?} (the -t info mode uses -n DOMAIN instead)",
            n.kind
        )
    })?;
    net::cmd_net_t(
        input,
        n.kind,
        n.output.as_deref(),
        n.timeout,
        &n.exclude,
        &n.sort,
        n.threads,
        &n.words,
    )
}

fn run_task(t: TaskArgs) -> anyhow::Result<()> {
    match t.cmd {
        Some(TaskCmd::List { name, port }) => task::cmd_list(name.as_deref(), port.or(t.port)),
        Some(TaskCmd::Kill { target, force }) => {
            if t.port.is_some() {
                anyhow::bail!("`-o/--port` applies to the list mode only (e.g. `sys task -o 8080`)");
            }
            task::cmd_kill(&target, force)
        }
        // `sys task` with no subcommand defaults to listing every process.
        None => task::cmd_list(None, t.port),
    }
}

fn run_http(h: HttpArgs) -> anyhow::Result<i32> {
    if h.help {
        httpie::print_help();
        return Ok(0);
    }
    let cfg = httpie::HttpConfig {
        json: h.json,
        form: h.form,
        multipart: h.multipart,
        raw: h.raw,
        file: h.file,
        print: h.print,
        headers_only: h.headers,
        body_only: h.body,
        meta_only: h.meta,
        verbose: h.verbose,
        output: h.output,
        download: h.download,
        quiet: h.quiet,
        pretty: h.pretty,
        auth: h.auth,
        auth_type: h.auth_type,
        proxies: h.proxy,
        follow: h.follow,
        max_redirects: h.max_redirects,
        timeout: h.timeout,
        check_status: h.check_status,
        offline: h.offline,
        verify: h.verify,
        ignore_stdin: h.ignore_stdin,
        default_scheme: h.default_scheme.unwrap_or_else(|| "http".to_string()),
        debug: h.debug,
        args: h.args,
    };
    httpie::run(&cfg)
}

mod ai;
mod chat;
mod env;
mod httpie;
mod link;
mod path;
mod short;
mod store;
mod task;

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "sysenv",
    version = concat!(env!("CARGO_PKG_VERSION"), " (Made by Gary-china)"),
    about = "System PATH & environment manager + httpie-compatible HTTP client (Windows / Ubuntu)",
    long_about = "sysenv manages the system PATH and environment variables (persisted and applied to the current environment), imports/exports PATH to the registry, links executables into a PATH directory, queries the DataLearner AI model list and the models.dev database of AI providers, chats with LLM providers configured in ~/.sysenv/config.yaml, installs command shims (spath/senv/slink/shttp/sai), and ships an httpie-compatible HTTP client.

Examples:
  sysenv path list
  sysenv path add D:\\tools
  sysenv path export backup.reg
  sysenv env set FOO bar
  sysenv link myapp.exe
  sysenv ai model gpt-4.1
  sysenv ai cn-model gpt-6.1-sol
  sysenv ai chat \"你好\"
  sysenv ai task -t weather country:北京
  sysenv ai provider openai
  sysenv task list
  sysenv task kill 1234
  sysenv short
  sysenv http pie.dev/get name=John"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage PATH entries: list / add / remove / has / export / import
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

Usage: sysenv http [flags] [METHOD] URL [ITEM...]

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

Use `sysenv help http` for help (in http subcommand, -h means response headers)."
    )]
    Http(HttpArgs),
    /// Query AI model info (DataLearner), provider info (models.dev), chat with configured providers
    Ai(AiArgs),
    /// Query and kill processes (list PID/name/path; kill by PID or name; no args lists all)
    Task(TaskArgs),
    /// Install shell shims for every subcommand (spath/senv/slink/shttp/sai/stask)
    Short(ShortArgs),
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
    /// Add a directory to PATH (persisted by default; affects new processes and this process)
    Add {
        /// Directory to add (relative paths are resolved to absolute)
        dir: String,
        /// Prepend instead of append
        #[arg(short = 'p', long)]
        prepend: bool,
        /// Do not persist; print a snippet to apply to the current shell only
        #[arg(long)]
        temporary: bool,
        /// Scope of the change: user (default) or machine (admin)
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
    /// Chat with an LLM configured in ~/.sysenv/config.yaml (OpenAI / Anthropic compatible)
    Chat(ChatArgs),
    /// Run a chat task template from the config (no -t lists the tasks)
    Task(AiTaskArgs),
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
    /// Show only models with open_weights enabled
    #[arg(long)]
    open: bool,
    /// Print the raw JSON (as a JSON array) instead of the formatted view
    #[arg(long)]
    json: bool,
    /// Output format: json (JSON array) or csv (table); default is a formatted text view
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
    /// Maximum number of entries to show for --search results (default 20)
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
}

#[derive(Args)]
struct AiTaskArgs {
    /// Task name from the config (e.g. -t weather); omit to list the available tasks
    #[arg(short = 't', long, value_name = "NAME")]
    task: Option<String>,
    /// Template parameters like key:value or key=value (e.g. country:北京)
    #[arg(value_name = "PARAM")]
    params: Vec<String>,
    /// Config file path (default: ~/.sysenv/config.yaml)
    #[arg(short = 'c', long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Print the actual HTTP request (method/URL/headers/body) and response (status/headers/body)
    #[arg(long)]
    debug: bool,
    /// Disable streaming even if the config sets stream: true
    #[arg(long)]
    no_stream: bool,
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

/// Machine-readable output formats for `sysenv ai`.
#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
enum OutFormat {
    /// JSON array output
    Json,
    /// CSV table output
    Csv,
}

#[derive(Args)]
struct TaskArgs {
    /// Show only the process using PORT (list mode, e.g. `sysenv task -o 8080`)
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
    /// Print parameter descriptions and examples, then exit (use `sysenv help http` for clap help)
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
        Cmd::Task(t) => run_task(t).map(|_| 0),
        Cmd::Short(s) => run_short(s).map(|_| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("sysenv: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_path(p: PathArgs) -> anyhow::Result<()> {
    match p.cmd {
        PathCmd::List => path::cmd_list(),
        PathCmd::Add { dir, prepend, temporary, scope } => {
            path::cmd_add(&dir, scope, prepend, temporary)
        }
        PathCmd::Remove { dir, temporary, scope } => path::cmd_remove(&dir, scope, temporary),
        PathCmd::Has { dir } => path::cmd_has(&dir),
        PathCmd::Export { file } => path::cmd_export(&file),
        PathCmd::Import { file, replace, scope } => path::cmd_import(&file, scope, replace),
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
        ),
        AiCmd::CnModel(m) => ai::cmd_cn_model(
            m.name.as_deref(),
            m.search.as_deref(),
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
        AiCmd::Chat(c) => chat::cmd_chat(
            &c.msg,
            c.config.as_deref(),
            c.debug,
            c.no_stream,
            c.model.as_deref(),
            c.list_model,
            c.list_provider,
        ),
        AiCmd::Task(t) => chat::cmd_task(
            t.task.as_deref(),
            &t.params,
            t.config.as_deref(),
            t.debug,
            t.no_stream,
        ),
    }
}

fn run_short(s: ShortArgs) -> anyhow::Result<()> {
    short::cmd_short(s.dir, s.force, s.temporary)
}

fn run_task(t: TaskArgs) -> anyhow::Result<()> {
    match t.cmd {
        Some(TaskCmd::List { name, port }) => task::cmd_list(name.as_deref(), port.or(t.port)),
        Some(TaskCmd::Kill { target, force }) => {
            if t.port.is_some() {
                anyhow::bail!("`-o/--port` applies to the list mode only (e.g. `sysenv task -o 8080`)");
            }
            task::cmd_kill(&target, force)
        }
        // `sysenv task` with no subcommand defaults to listing every process.
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

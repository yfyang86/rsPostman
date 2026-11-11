//! `rspostman` command line interface.

#![allow(clippy::large_enum_variant)]

mod out;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use rspostman_core::http::Settings;
use rspostman_core::import::Imported;
use rspostman_core::model::{Auth, Body, Collection, Environment, GraphQl, Item, Request, KV};
use rspostman_core::{codegen, engine, Exchange, Session, Workspace};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rspostman", version, about = "rsPostman - a local-first, Postman-compatible API client")]
struct Cli {
    /// Workspace directory (default: $RSPOSTMAN_HOME or ~/.rspostman)
    #[arg(short, long, global = true)]
    workspace: Option<PathBuf>,
    /// Disable coloured output
    #[arg(long, global = true)]
    no_color: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone, Default)]
struct RequestArgs {
    /// Header, repeatable
    #[arg(short = 'H', long = "header", value_name = "NAME:VALUE")]
    headers: Vec<String>,
    /// Query parameter, repeatable
    #[arg(short = 'q', long = "query", value_name = "KEY=VALUE")]
    query: Vec<String>,
    /// Raw body (`@file` reads the text from a file)
    #[arg(short = 'd', long)]
    data: Option<String>,
    /// JSON body (sets Content-Type: application/json)
    #[arg(long)]
    json: Option<String>,
    /// x-www-form-urlencoded field, repeatable
    #[arg(long = "form", value_name = "KEY=VALUE")]
    form: Vec<String>,
    /// multipart/form-data field, repeatable (`key=@path` uploads a file)
    #[arg(short = 'F', long = "multipart", value_name = "KEY=VALUE|KEY=@FILE")]
    multipart: Vec<String>,
    /// Send a file as binary body
    #[arg(long, value_name = "FILE")]
    body_file: Option<String>,
    /// GraphQL query
    #[arg(long)]
    graphql: Option<String>,
    /// GraphQL variables (JSON)
    #[arg(long, value_name = "JSON")]
    graphql_vars: Option<String>,
    /// Basic auth. Give only USER to be asked for the password (or set RSPOSTMAN_PASSWORD);
    /// `USER:env:NAME` reads it from an environment variable
    #[arg(short = 'u', long = "user", value_name = "USER[:PASS]")]
    user: Option<String>,
    /// Bearer token: the literal, `env:NAME`, `@FILE`, or `-` to be asked for it
    #[arg(long, value_name = "TOKEN")]
    bearer: Option<String>,
    /// API key sent as header; the value may be `env:NAME`, `@FILE` or `-` like --bearer
    #[arg(long, value_name = "NAME=VALUE")]
    api_key: Option<String>,
    /// Auth type (noauth, basic, bearer, apikey, digest, oauth1, oauth2, awsv4, jwt) used with --auth-param
    #[arg(long, value_name = "TYPE")]
    auth: Option<String>,
    /// Auth parameter for --auth, repeatable (e.g. username=bob); secret values may be `env:NAME`
    #[arg(long, value_name = "KEY=VALUE")]
    auth_param: Vec<String>,
}

#[derive(Args, Clone, Default)]
struct ExecArgs {
    /// Environment (name, id or exported file); default: the active environment
    #[arg(short, long)]
    env: Option<String>,
    /// Override a variable for this invocation, repeatable
    #[arg(long = "var", value_name = "KEY=VALUE")]
    vars: Vec<String>,
    /// Show response headers
    #[arg(short, long)]
    include: bool,
    /// Show the request that was sent and response headers
    #[arg(short, long)]
    verbose: bool,
    /// Write the response body to a file
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Do not pretty-print the body
    #[arg(long)]
    raw: bool,
    /// Do not record this request in the history
    #[arg(long)]
    no_history: bool,
    /// Exit with status 22 when the HTTP status is >= 400
    #[arg(long)]
    fail: bool,
    /// Skip TLS certificate verification
    #[arg(short = 'k', long)]
    insecure: bool,
    /// Timeout in milliseconds
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
    /// Do not follow redirects
    #[arg(long)]
    no_redirect: bool,
    /// Print credentials in verbose output instead of `<redacted>`
    #[arg(long)]
    show_secrets: bool,
}

#[derive(Args, Clone, Default)]
struct RunArgs {
    /// Run only this folder or request (id, name or path)
    #[arg(long)]
    folder: Option<String>,
    /// Environment (name, id or exported file); default: the active environment
    #[arg(short, long)]
    env: Option<String>,
    /// Override a variable, repeatable
    #[arg(long = "var", value_name = "KEY=VALUE")]
    vars: Vec<String>,
    /// Number of iterations (default: number of data rows, or 1)
    #[arg(short = 'n', long)]
    iterations: Option<usize>,
    /// Iteration data file (CSV with header row, or JSON array of objects)
    #[arg(short, long, value_name = "FILE")]
    data: Option<String>,
    /// Delay between requests in milliseconds
    #[arg(long, default_value_t = 0, value_name = "MS")]
    delay: u64,
    /// Stop at the first failure
    #[arg(long)]
    bail: bool,
    /// Do not save variable changes made by scripts back to the workspace
    #[arg(long)]
    no_persist: bool,
    /// Skip TLS certificate verification
    #[arg(short = 'k', long)]
    insecure: bool,
    /// Request timeout in milliseconds
    #[arg(long, value_name = "MS")]
    timeout: Option<u64>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Send an ad-hoc request: `send [METHOD] URL`
    Send {
        /// `[METHOD] URL`
        #[arg(required = true, num_args = 1..=2, value_name = "[METHOD] URL")]
        target: Vec<String>,
        #[command(flatten)]
        req: RequestArgs,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Send a request saved in a collection
    #[command(alias = "req")]
    Request {
        collection: String,
        /// Request id, name or `Folder/Name` path
        request: String,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Manage collections
    #[command(alias = "c", subcommand)]
    Collection(CollectionCmd),
    /// Manage environments
    #[command(subcommand)]
    Env(EnvCmd),
    /// Manage global variables
    #[command(subcommand)]
    Globals(VarCmd),
    /// Request history
    #[command(subcommand)]
    History(HistoryCmd),
    /// Cookie jar
    #[command(subcommand)]
    Cookies(CookiesCmd),
    /// Import a Postman collection / environment, OpenAPI / Swagger, HAR file or a curl command (`-` reads stdin)
    Import {
        /// File path, `-` for stdin, or a literal curl command
        source: String,
        /// Target collection for imported curl requests (created when missing)
        #[arg(short, long)]
        collection: Option<String>,
        /// Name for an imported curl request
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Export a collection, environment or the globals as Postman JSON
    #[command(subcommand)]
    Export(ExportCmd),
    /// Generate a code snippet (`codegen --list` shows the languages)
    Codegen {
        /// Target language
        #[arg(default_value = "curl")]
        lang: String,
        /// List supported languages
        #[arg(long)]
        list: bool,
        /// Collection containing the request
        #[arg(short, long, requires = "request")]
        collection: Option<String>,
        /// Request id, name or path in the collection
        #[arg(short, long, requires = "collection")]
        request: Option<String>,
        /// Ad-hoc request: `[METHOD] URL`
        #[arg(num_args = 0..=2, value_name = "[METHOD] URL")]
        target: Vec<String>,
        #[command(flatten)]
        req: RequestArgs,
        #[arg(short, long)]
        env: Option<String>,
    },
    /// Run a collection (or folder) with its scripts and tests, like Newman
    Run {
        /// Collection id, name or exported file
        collection: String,
        #[command(flatten)]
        run: RunArgs,
        /// Write a JSON report
        #[arg(long, value_name = "FILE")]
        report_json: Option<PathBuf>,
        /// Write a JUnit XML report
        #[arg(long, value_name = "FILE")]
        report_junit: Option<PathBuf>,
        /// Write an HTML report
        #[arg(long, value_name = "FILE")]
        report_html: Option<PathBuf>,
        /// Only print the summary
        #[arg(short, long)]
        quiet: bool,
        /// Print console output and response bodies
        #[arg(short, long)]
        verbose: bool,
    },
    /// Benchmark a request or every request of a collection: latency percentiles, time to first
    /// byte, requests per second, status codes and errors. Can run for a fixed time, at a fixed
    /// rate, check thresholds and compare with a baseline. Scripts are not run
    #[command(alias = "bench")]
    Benchmark {
        /// Ad-hoc request: `[METHOD] URL`
        #[arg(num_args = 0..=2, value_name = "[METHOD] URL")]
        target: Vec<String>,
        /// Collection id, name or exported file (omit for an ad-hoc request)
        #[arg(long)]
        collection: Option<String>,
        /// Request id, name or `Folder/Name` path (requires a collection)
        #[arg(short, long, requires = "collection")]
        request: Option<String>,
        /// Benchmark only this folder, id, name or path (requires a collection)
        #[arg(long, conflicts_with = "request")]
        folder: Option<String>,
        #[command(flatten)]
        req: RequestArgs,
        /// Environment (name, id or exported file); default: the active environment
        #[arg(short, long)]
        env: Option<String>,
        /// Iterations of each request
        #[arg(short = 'n', long, default_value_t = 10, conflicts_with = "duration")]
        iterations: usize,
        /// Run for a fixed time instead of a number of iterations, e.g. `30s`, `2m`, `500ms`
        #[arg(short = 't', long, value_name = "TIME")]
        duration: Option<String>,
        /// Number of concurrent workers (at most 256)
        #[arg(short = 'c', long, default_value_t = 1)]
        concurrency: usize,
        /// Target request rate in requests per second (all workers together)
        #[arg(long, value_name = "RPS")]
        rate: Option<f64>,
        /// Delay between the requests of each worker, in milliseconds
        #[arg(long, default_value_t = 0, value_name = "MS", conflicts_with = "rate")]
        delay: u64,
        /// Unmeasured warm-up rounds before the measurement starts
        #[arg(long, default_value_t = 0, value_name = "N")]
        warmup: usize,
        /// Open a new connection for every request instead of reusing connections
        #[arg(long)]
        no_keepalive: bool,
        /// Pass/fail condition, repeatable: `p95<200ms`, `error_rate<=1%`, `rps>=50`, `ttfb_p99<1s` …
        /// With thresholds the exit code follows them
        #[arg(long = "assert", value_name = "CONDITION")]
        asserts: Vec<String>,
        /// Compare with a JSON report of an earlier run; exit 1 when this run is worse
        #[arg(long, value_name = "FILE")]
        compare: Option<PathBuf>,
        /// Allowed slow-down (latency) or drop (requests/s) against the baseline, in percent
        #[arg(long, default_value_t = 10.0, value_name = "PERCENT")]
        tolerance: f64,
        /// Skip TLS certificate verification
        #[arg(short = 'k', long)]
        insecure: bool,
        /// Request timeout in milliseconds
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
        /// Write a JSON report (usable later as a --compare baseline)
        #[arg(long, value_name = "FILE")]
        report_json: Option<PathBuf>,
        /// Write every sample as CSV
        #[arg(long, value_name = "FILE")]
        report_csv: Option<PathBuf>,
        /// Write a standalone HTML report with charts
        #[arg(long, value_name = "FILE")]
        report_html: Option<PathBuf>,
        /// Only print the summary
        #[arg(long)]
        quiet: bool,
    },
    /// Profile an LLM provider: streaming health, time to first token, output rate and token usage
    Profile {
        /// `[METHOD] URL` of the completions endpoint (OpenAI-compatible)
        #[arg(required = true, num_args = 1..=2, value_name = "[METHOD] URL")]
        target: Vec<String>,
        #[command(flatten)]
        req: RequestArgs,
        /// Model for the generated chat body
        #[arg(long, default_value = "gpt-4o-mini", value_name = "NAME")]
        model: String,
        /// User prompt for the generated chat body
        #[arg(long, default_value = "Say hello in a few words.", value_name = "TEXT")]
        prompt: String,
        /// Max tokens for the generated chat body
        #[arg(long, value_name = "N")]
        max_tokens: Option<u32>,
        /// Number of profiling runs (default: 3)
        #[arg(short = 'n', long, default_value_t = 3)]
        iterations: usize,
        /// Delay between runs in milliseconds
        #[arg(long, default_value_t = 0, value_name = "MS")]
        delay: u64,
        /// Environment (name, id or exported file); default: the active environment
        #[arg(short, long)]
        env: Option<String>,
        /// Show the captured completions
        #[arg(short, long)]
        verbose: bool,
        /// Skip TLS certificate verification
        #[arg(short = 'k', long)]
        insecure: bool,
        /// Request timeout in milliseconds
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
        /// Write a JSON report
        #[arg(long, value_name = "FILE")]
        report_json: Option<PathBuf>,
    },
    /// Serve the saved examples of a collection as a local mock server
    Mock {
        collection: String,
        #[arg(short, long, default_value_t = 4010)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(short, long)]
        env: Option<String>,
        /// Delay every response by this many milliseconds
        #[arg(long, default_value_t = 0)]
        delay: u64,
        /// Do not add CORS headers
        #[arg(long)]
        no_cors: bool,
    },
    /// Run a collection on a schedule and log the results (local monitor)
    Monitor {
        collection: String,
        /// Interval between runs, e.g. `30s`, `5m`, `1h`
        #[arg(long, default_value = "5m")]
        every: String,
        /// Stop after this many runs (default: run until interrupted)
        #[arg(long)]
        count: Option<usize>,
        #[command(flatten)]
        run: RunArgs,
    },
    /// Open a WebSocket connection (interactive, or send -m messages and print the replies)
    Ws {
        url: String,
        #[arg(short = 'H', long = "header", value_name = "NAME:VALUE")]
        headers: Vec<String>,
        /// Message to send, repeatable; without it lines are read from stdin
        #[arg(short, long = "message")]
        messages: Vec<String>,
        /// How long to wait for replies after the last -m message (milliseconds)
        #[arg(long, default_value_t = 1000)]
        wait: u64,
        #[arg(short, long)]
        env: Option<String>,
    },
    /// Generate API documentation from a collection
    Docs {
        collection: String,
        /// `md` or `html`
        #[arg(short, long, default_value = "md")]
        format: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Start the graphical interface (local web UI) and open it in the browser
    Gui {
        #[arg(short, long, default_value_t = 8686)]
        port: u16,
        /// Do not open the browser automatically
        #[arg(long)]
        no_open: bool,
    },
    /// Encrypted local vault for secrets, referenced in requests as `{{vault:NAME}}`
    #[command(subcommand)]
    Vault(VaultCmd),
    /// Show or change settings
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Run the built-in echo (httpbin-like) test server
    Echo {
        #[arg(short, long, default_value_t = 8642)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
}

#[derive(Subcommand)]
enum CollectionCmd {
    /// List collections
    #[command(alias = "ls")]
    List,
    /// Create an empty collection
    Create { name: String, #[arg(short, long)] description: Option<String> },
    /// Show the tree of a collection
    Show { collection: String },
    /// Delete a collection
    #[command(alias = "rm")]
    Delete { collection: String },
    /// Rename a collection
    Rename { collection: String, name: String },
    /// Duplicate a collection
    Duplicate { collection: String, #[arg(short, long)] name: Option<String> },
    /// Add a folder
    AddFolder {
        collection: String,
        name: String,
        /// Parent folder (id, name or path)
        #[arg(short, long)]
        parent: Option<String>,
    },
    /// Add a request: `add-request <collection> <name> [METHOD] URL`
    AddRequest {
        collection: String,
        name: String,
        #[arg(required = true, num_args = 1..=2, value_name = "[METHOD] URL")]
        target: Vec<String>,
        #[arg(short, long)]
        parent: Option<String>,
        #[command(flatten)]
        req: RequestArgs,
    },
    /// Remove a folder or request
    Remove { collection: String, item: String },
    /// Move an item into another folder (omit --to for the collection root)
    Move { collection: String, item: String, #[arg(long)] to: Option<String>, #[arg(long)] index: Option<usize> },
    /// Rename a folder or request
    RenameItem { collection: String, item: String, name: String },
    /// Set a collection variable (omit VALUE to be asked for it without echo)
    SetVar {
        collection: String,
        key: String,
        value: Option<String>,
        /// Mark the variable as secret: masked in listings and in the GUI
        #[arg(long)]
        secret: bool,
    },
    /// Remove a collection variable
    UnsetVar { collection: String, key: String },
    /// Set the auth of the collection or of one item (`--auth noauth`, `--auth inherit` ...)
    SetAuth {
        collection: String,
        /// Folder or request; omit for the collection itself
        #[arg(long)]
        item: Option<String>,
        #[arg(long = "type", value_name = "TYPE")]
        kind: String,
        #[arg(long = "param", value_name = "KEY=VALUE")]
        params: Vec<String>,
    },
}

#[derive(Subcommand)]
enum EnvCmd {
    /// List environments (the active one is marked with *)
    #[command(alias = "ls")]
    List,
    Create { name: String },
    Show {
        env: String,
        /// Print the values of secret variables
        #[arg(long)]
        show_secrets: bool,
    },
    #[command(alias = "rm")]
    Delete { env: String },
    Rename { env: String, name: String },
    /// Set a variable
    Set {
        env: String,
        key: String,
        /// Omit to be asked for the value without echo (or pipe it on stdin)
        value: Option<String>,
        /// Mark the variable as secret: masked in listings and in the GUI
        #[arg(long)]
        secret: bool,
    },
    /// Remove a variable
    Unset { env: String, key: String },
    /// Select the active environment (`none` clears it)
    Use { env: String },
}

#[derive(Subcommand)]
enum VarCmd {
    #[command(alias = "ls")]
    List {
        /// Print the values of secret variables
        #[arg(long)]
        show_secrets: bool,
    },
    /// Set a variable (omit VALUE to be asked for it without echo)
    Set {
        key: String,
        value: Option<String>,
        /// Mark the variable as secret: masked in listings and in the GUI
        #[arg(long)]
        secret: bool,
    },
    Unset { key: String },
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// List recent requests (most recent first)
    #[command(alias = "ls")]
    List { #[arg(short = 'n', long, default_value_t = 20)] limit: usize },
    /// Show one entry (number from `history list`, or id prefix)
    Show { entry: String },
    /// Send an entry again
    Replay {
        entry: String,
        #[command(flatten)]
        exec: ExecArgs,
    },
    /// Delete the whole history
    Clear,
}

#[derive(Subcommand)]
enum CookiesCmd {
    #[command(alias = "ls")]
    List,
    /// Set a cookie for a URL, e.g. `cookies set http://localhost:3000 "sid=1; Path=/"`
    Set { url: String, cookie: String },
    /// Remove all cookies, the cookies of one domain, or one cookie
    Clear { domain: Option<String>, name: Option<String> },
}

#[derive(Subcommand)]
enum ExportCmd {
    Collection { collection: String, #[arg(short, long)] output: Option<PathBuf> },
    Env { env: String, #[arg(short, long)] output: Option<PathBuf> },
    Globals { #[arg(short, long)] output: Option<PathBuf> },
}

#[derive(Subcommand)]
enum VaultCmd {
    /// Create the vault (asks for a new passphrase, or takes it from RSPOSTMAN_VAULT_PASSPHRASE)
    Init,
    /// Show whether a vault exists and how many secrets it holds
    Status,
    /// List secret names (does not need the passphrase)
    #[command(alias = "ls")]
    List,
    /// Store a secret; omit VALUE to be asked for it without echo (or pipe it on stdin).
    /// Creates the vault on first use
    Set { name: String, value: Option<String> },
    /// Print a secret
    Get { name: String },
    /// Delete a secret
    #[command(alias = "rm")]
    Remove { name: String },
    /// Change the passphrase (the new one is asked for, or taken from RSPOSTMAN_VAULT_NEW_PASSPHRASE)
    Passwd,
}

#[derive(Subcommand)]
enum SettingsCmd {
    Show,
    /// Set a value, e.g. `settings set timeout_ms 5000`
    Set { key: String, value: String },
    Reset,
}

fn kv(s: &str, sep: char, what: &str) -> Result<(String, String)> {
    s.split_once(sep).map(|(k, v)| (k.trim().to_string(), v.trim_start().to_string())).ok_or_else(|| anyhow!("{what} must look like KEY{sep}VALUE, got `{s}`"))
}

fn target(parts: &[String], has_body: bool) -> Result<(String, String)> {
    match parts {
        [url] => Ok((if has_body { "POST" } else { "GET" }.to_string(), url.clone())),
        [m, url] => Ok((m.to_uppercase(), url.clone())),
        _ => bail!("expected `[METHOD] URL`"),
    }
}

/// Ask for a secret without echoing it (reads one line from stdin when it is not a terminal).
fn prompt_secret(what: &str) -> Result<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password(format!("{what}: ")).with_context(|| format!("cannot read {what}"));
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).with_context(|| format!("cannot read {what} from stdin"))?;
    let value = line.trim_end_matches(['\r', '\n']).to_string();
    if value.is_empty() {
        bail!("no {what} given (stdin was empty)");
    }
    Ok(value)
}

const VAULT_ENV: &str = "RSPOSTMAN_VAULT_PASSPHRASE";
const VAULT_NEW_ENV: &str = "RSPOSTMAN_VAULT_NEW_PASSPHRASE";

/// The vault passphrase: environment variable, else a hidden prompt on a terminal.
/// It is never read from piped stdin, which may carry a secret value instead.
fn vault_passphrase(env: &str, prompt: &str) -> Result<String> {
    use std::io::IsTerminal;
    if let Ok(p) = std::env::var(env) {
        return Ok(p);
    }
    if !std::io::stdin().is_terminal() {
        bail!("the vault passphrase is needed: set {env} or run in a terminal to be asked for it");
    }
    rpassword::prompt_password(format!("{prompt}: ")).context("cannot read the passphrase")
}

/// A passphrase for a new vault: from the environment, or asked twice.
fn new_vault_passphrase(env: &str) -> Result<String> {
    use std::io::IsTerminal;
    if let Ok(p) = std::env::var(env) {
        return Ok(p);
    }
    if !std::io::stdin().is_terminal() {
        bail!("a new vault passphrase is needed: set {env} or run in a terminal to be asked for it");
    }
    let first = rpassword::prompt_password("New vault passphrase: ").context("cannot read the passphrase")?;
    let second = rpassword::prompt_password("Repeat the passphrase: ").context("cannot read the passphrase")?;
    if first != second {
        bail!("the passphrases do not match");
    }
    Ok(first)
}

fn open_vault(ws: &Workspace) -> Result<rspostman_core::vault::Vault> {
    if !rspostman_core::vault::exists(ws) {
        bail!("this workspace has no vault yet; create one with `rspostman vault init` or `rspostman vault set NAME`");
    }
    rspostman_core::vault::Vault::unlock(ws, &vault_passphrase(VAULT_ENV, "Vault passphrase")?)
}

/// Load a session and, when anything refers to `{{vault:…}}` (the collection, variables or
/// the command line itself), unlock the vault for it.
fn load_session(ws: &Workspace, collection: Option<&Collection>, env: Option<&str>) -> Result<Session> {
    let mut session = Session::load(ws, collection, env)?;
    let mut text = std::env::args().collect::<Vec<_>>().join("\n");
    if let Some(c) = collection {
        text.push_str(&c.to_json());
    }
    for scope in [&session.scopes.globals, &session.scopes.collection, &session.scopes.environment] {
        for v in scope.values() {
            text.push_str(v);
            text.push('\n');
        }
    }
    if !rspostman_core::vault::references(&text).is_empty() {
        session.unlock_vault(&open_vault(ws)?)?;
    }
    Ok(session)
}

fn vault_cmd(ws: &Workspace, cmd: VaultCmd) -> Result<i32> {
    use rspostman_core::vault::{self, Vault};
    match cmd {
        VaultCmd::Init => {
            if vault::exists(ws) {
                bail!("a vault already exists in this workspace (use `rspostman vault passwd` to change its passphrase)");
            }
            Vault::create(ws, &new_vault_passphrase(VAULT_ENV)?)?;
            println!("vault created at {}", vault::path(ws).display());
            println!("There is no way to recover the passphrase: keep it somewhere safe.");
        }
        VaultCmd::Status => {
            if vault::exists(ws) {
                println!("vault: {} ({} secret(s), locked until the passphrase is given)", vault::path(ws).display(), vault::names(ws)?.len());
            } else {
                println!("no vault in this workspace; create one with `rspostman vault init`");
            }
        }
        VaultCmd::List => {
            let names = vault::names(ws)?;
            if names.is_empty() {
                println!("{}", if vault::exists(ws) { "The vault is empty." } else { "No vault in this workspace." });
            }
            for n in names {
                println!("{n}");
            }
        }
        VaultCmd::Set { name, value } => {
            if !vault::valid_name(&name) {
                bail!("invalid secret name `{name}` (use letters, digits, `_`, `-` and `.`)");
            }
            let mut v = if vault::exists(ws) {
                open_vault(ws)?
            } else {
                eprintln!("No vault yet – creating one.");
                Vault::create(ws, &new_vault_passphrase(VAULT_ENV)?)?
            };
            let value = match value {
                Some(v) => v,
                None => prompt_secret(&format!("value for {name}"))?,
            };
            v.set(&name, &value)?;
            println!("stored `{name}`; use it as {{{{vault:{name}}}}}");
        }
        VaultCmd::Get { name } => match open_vault(ws)?.get(&name)? {
            Some(v) => println!("{v}"),
            None => bail!("the vault has no secret named `{name}`"),
        },
        VaultCmd::Remove { name } => {
            if !open_vault(ws)?.remove(&name)? {
                bail!("the vault has no secret named `{name}`");
            }
            println!("removed `{name}`");
        }
        VaultCmd::Passwd => {
            let mut v = open_vault(ws)?;
            v.change_passphrase(&new_vault_passphrase(VAULT_NEW_ENV)?)?;
            println!("passphrase changed; all secrets were re-encrypted");
        }
    }
    Ok(0)
}

/// Resolve a credential argument so that it does not have to appear on the command line:
/// `env:NAME` reads an environment variable, `-` asks for it, and – when `allow_file` –
/// `@FILE` reads a file. Anything else is taken literally.
fn secret_arg(value: &str, what: &str, allow_file: bool) -> Result<String> {
    if let Some(name) = value.strip_prefix("env:") {
        return std::env::var(name).map_err(|_| anyhow!("environment variable `{name}` (for {what}) is not set"));
    }
    if value == "-" {
        return prompt_secret(what);
    }
    if let (true, Some(path)) = (allow_file, value.strip_prefix('@')) {
        let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {what} from {path}"))?;
        return Ok(text.trim().to_string());
    }
    Ok(value.to_string())
}

fn build_auth(kind: &str, params: &[String]) -> Result<Option<Auth>> {
    if kind == "inherit" {
        return Ok(None);
    }
    if !rspostman_core::auth::AUTH_TYPES.contains(&kind) {
        bail!("unknown auth type `{kind}` (supported: {})", rspostman_core::auth::AUTH_TYPES.join(", "));
    }
    let mut a = Auth { kind: kind.to_string(), ..Default::default() };
    for p in params {
        let (k, v) = kv(p, '=', "auth parameter")?;
        let v = if rspostman_core::secrets::SECRET_AUTH_PARAMS.contains(&k.as_str()) { secret_arg(&v, &k, false)? } else { v };
        a.params.insert(k, v);
    }
    Ok(Some(a))
}

fn build_request(parts: &[String], a: &RequestArgs) -> Result<Request> {
    let has_body = a.data.is_some() || a.json.is_some() || !a.form.is_empty() || !a.multipart.is_empty() || a.body_file.is_some() || a.graphql.is_some();
    let (method, url) = target(parts, has_body)?;
    let mut req = Request::new(&method, &url);
    for q in &a.query {
        let (k, v) = kv(q, '=', "query parameter")?;
        req.url.query.push(KV::new(k, v));
    }
    if !a.query.is_empty() {
        req.url.sync_raw_from_query();
    }
    for h in &a.headers {
        let (k, v) = kv(h, ':', "header")?;
        req.header.push(KV::new(k, v));
    }
    let read = |s: &str| -> Result<String> {
        match s.strip_prefix('@') {
            Some(p) => std::fs::read_to_string(p).with_context(|| format!("cannot read {p}")),
            None => Ok(s.to_string()),
        }
    };
    if let Some(j) = &a.json {
        req.body = Some(Body::raw(&read(j)?, "json"));
    } else if let Some(d) = &a.data {
        let text = read(d)?;
        let lang = if serde_json::from_str::<serde_json::Value>(&text).map(|v| v.is_object() || v.is_array()).unwrap_or(false) { "json" } else { "text" };
        req.body = Some(Body::raw(&text, lang));
    } else if !a.form.is_empty() {
        let mut fields = vec![];
        for f in &a.form {
            let (k, v) = kv(f, '=', "form field")?;
            fields.push(KV::new(k, v));
        }
        req.body = Some(Body { mode: "urlencoded".into(), urlencoded: fields, ..Default::default() });
    } else if !a.multipart.is_empty() {
        let mut fields = vec![];
        for f in &a.multipart {
            let (k, v) = kv(f, '=', "multipart field")?;
            let mut row = KV::new(k, "");
            match v.strip_prefix('@') {
                Some(path) => {
                    row.kind = Some("file".into());
                    row.src = Some(serde_json::json!(path));
                }
                None => {
                    row.kind = Some("text".into());
                    row.value = v;
                }
            }
            fields.push(row);
        }
        req.body = Some(Body { mode: "formdata".into(), formdata: fields, ..Default::default() });
    } else if let Some(f) = &a.body_file {
        req.body = Some(Body { mode: "file".into(), file: Some(serde_json::json!({ "src": f })), ..Default::default() });
    } else if let Some(g) = &a.graphql {
        req.body = Some(Body {
            mode: "graphql".into(),
            graphql: Some(GraphQl { query: read(g)?, variables: a.graphql_vars.clone().unwrap_or_default() }),
            ..Default::default()
        });
    }
    if let Some(u) = &a.user {
        let (name, pass) = match u.split_once(':') {
            Some((name, pass)) => (name.to_string(), secret_arg(pass, "password", false)?),
            // no password on the command line: environment variable, else ask
            None => match std::env::var("RSPOSTMAN_PASSWORD") {
                Ok(pw) => (u.clone(), pw),
                Err(_) => (u.clone(), prompt_secret(&format!("password for {u}"))?),
            },
        };
        req.auth = Some(Auth::new("basic", &[("username", &name), ("password", &pass)]));
    } else if let Some(t) = &a.bearer {
        req.auth = Some(Auth::new("bearer", &[("token", &secret_arg(t, "bearer token", true)?)]));
    } else if let Some(k) = &a.api_key {
        let (name, value) = kv(k, '=', "--api-key")?;
        let value = secret_arg(&value, "API key", true)?;
        req.auth = Some(Auth::new("apikey", &[("key", &name), ("value", &value), ("in", "header")]));
    } else if let Some(t) = &a.auth {
        req.auth = build_auth(t, &a.auth_param)?;
    }
    Ok(req)
}

fn apply_exec(session: &mut Session, exec: &ExecArgs) -> Result<()> {
    for v in &exec.vars {
        let (k, val) = kv(v, '=', "--var")?;
        session.scopes.local.insert(k, val);
    }
    if exec.insecure {
        session.settings.verify_ssl = false;
    }
    if let Some(t) = exec.timeout {
        session.settings.timeout_ms = t;
    }
    if exec.no_redirect {
        session.settings.follow_redirects = false;
    }
    Ok(())
}

/// Print an exchange; returns the process exit code.
fn report(ex: &Exchange, exec: &ExecArgs) -> Result<i32> {
    if exec.verbose {
        if let Some(p) = &ex.request {
            if exec.show_secrets {
                out::request(p);
            } else {
                out::request(&rspostman_core::secrets::redact_prepared(p));
            }
        }
    }
    if let Some(e) = &ex.error {
        eprintln!("{} {e}", out::red("error:"));
        return Ok(1);
    }
    let Some(r) = &ex.response else { return Ok(1) };
    out::response_meta(r, exec.include || exec.verbose);
    match &exec.output {
        Some(path) => {
            std::fs::write(path, &r.body).with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("{}", out::dim(&format!("body saved to {}", path.display())));
        }
        None => {
            let text = out::body_text(r, exec.raw);
            if !text.is_empty() {
                println!("{text}");
            }
        }
    }
    print_script_results(ex, true);
    Ok(if ex.failed() {
        1
    } else if exec.fail && r.status >= 400 {
        22
    } else {
        0
    })
}

/// Console output and test results of one exchange (to stderr).
fn print_script_results(ex: &Exchange, console: bool) {
    if console {
        for l in &ex.console {
            eprintln!("  {} {}", out::dim(&format!("│ {}:", l.level)), l.message);
        }
    }
    for t in &ex.tests {
        if t.skipped {
            eprintln!("  {} {}", out::yellow("-"), out::dim(&format!("{} (skipped)", t.name)));
        } else if t.passed {
            eprintln!("  {} {}", out::green("✓"), t.name);
        } else {
            eprintln!("  {} {}", out::red("✗"), out::red(&t.name));
            eprintln!("      {}", out::dim(t.error.as_deref().unwrap_or("assertion failed")));
        }
    }
    if let Some(e) = &ex.script_error {
        eprintln!("  {} {}", out::red("✗"), out::red(e));
    }
}

fn load_run(ws: &Workspace, collection: &Collection, a: &RunArgs) -> Result<(Session, rspostman_core::runner::RunOptions)> {
    let mut session = load_session(ws, Some(collection), a.env.as_deref())?;
    for v in &a.vars {
        let (k, val) = kv(v, '=', "--var")?;
        session.scopes.local.insert(k, val);
    }
    if a.insecure {
        session.settings.verify_ssl = false;
    }
    if let Some(t) = a.timeout {
        session.settings.timeout_ms = t;
    }
    let opts = rspostman_core::runner::RunOptions {
        folder: match &a.folder {
            Some(f) => Some(resolve_item(collection, f)?.id.clone()),
            None => None,
        },
        item_ids: None,
        iterations: a.iterations,
        data: match &a.data {
            Some(path) => rspostman_core::runner::load_data(path)?,
            None => vec![],
        },
        delay_ms: a.delay,
        bail: a.bail,
    };
    Ok((session, opts))
}

fn print_summary(s: &rspostman_core::runner::RunSummary) {
    use rspostman_core::http::{human_size, human_time};
    let st = &s.stats;
    eprintln!();
    out::table(
        &["", "EXECUTED", "FAILED"],
        &[
            vec!["iterations".into(), st.iterations.to_string(), "-".into()],
            vec!["requests".into(), st.requests.to_string(), st.requests_failed.to_string()],
            vec!["assertions".into(), st.tests.to_string(), st.tests_failed.to_string()],
        ],
    );
    println!(
        "duration {}, average response time {}, received {}",
        human_time(s.duration_ms),
        human_time(if st.requests > 0 { st.response_time_ms / st.requests as u64 } else { 0 }),
        human_size(st.response_bytes)
    );
    if let Some(a) = &s.aborted {
        println!("{}", out::yellow(&format!("run ended early: {a}")));
    }
    let mut n = 0;
    for x in s.executions.iter().filter(|x| x.exchange.failed()) {
        if n == 0 {
            println!("\n{}", out::bold("Failures"));
        }
        let ex = &x.exchange;
        for err in ex.error.iter().chain(ex.script_error.iter()) {
            n += 1;
            println!("{n:>3}. {} (iteration {})\n     {}", out::red(&x.path), x.iteration + 1, err);
        }
        for t in ex.tests.iter().filter(|t| !t.passed) {
            n += 1;
            println!("{n:>3}. {} / {} (iteration {})\n     {}", out::red(&x.path), t.name, x.iteration + 1, t.error.as_deref().unwrap_or("assertion failed"));
        }
    }
}

fn parse_interval(s: &str) -> Result<u64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
    let n: u64 = num.parse().map_err(|_| anyhow!("invalid interval `{s}` (examples: 30s, 5m, 1h)"))?;
    Ok(match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "ms" => (n / 1000).max(1),
        _ => bail!("invalid interval unit in `{s}` (use s, m or h)"),
    })
}

/// One millisecond value for tables: keep one decimal for small numbers.
fn fmt_ms(x: f64) -> String {
    if x < 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.0}")
    }
}

/// `30s`, `2m`, `500ms`, `1h` -> milliseconds.
fn parse_duration_ms(s: &str) -> Result<u64> {
    let t = s.trim().to_lowercase();
    let (num, unit) = t.split_at(t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len()));
    let n: f64 = num.parse().map_err(|_| anyhow!("invalid duration `{s}` (examples: 30s, 2m, 500ms)"))?;
    let ms = match unit {
        "ms" => n,
        "" | "s" => n * 1000.0,
        "m" => n * 60_000.0,
        "h" => n * 3_600_000.0,
        _ => bail!("invalid duration unit in `{s}` (use ms, s, m or h)"),
    };
    if ms < 1.0 {
        bail!("the duration must be at least 1 ms");
    }
    Ok(ms as u64)
}

fn stats_cells(s: &rspostman_core::profile::LatencyStats) -> Vec<String> {
    [s.min, s.p10, s.p50, s.p75, s.p90, s.p95, s.p99, s.p999, s.max, s.mean].iter().map(|v| fmt_ms(*v)).collect()
}

fn print_bench_summary(r: &rspostman_core::bench::BenchReport, comparison: Option<&rspostman_core::bench::Comparison>) {
    use rspostman_core::http::{human_size, human_time};
    println!();
    match &r.overall {
        Some(s) => {
            let mut rows = vec![[vec!["total".to_string()], stats_cells(s)].concat()];
            if let Some(t) = &r.ttfb {
                rows.push([vec!["first byte".to_string()], stats_cells(t)].concat());
            }
            out::table(&["LATENCY (ms)", "MIN", "P10", "P50", "P75", "P90", "P95", "P99", "P99.9", "MAX", "MEAN"], &rows);
        }
        None => println!("{}", out::red("no request was answered")),
    }
    if r.by_name.len() > 1 {
        let rows: Vec<Vec<String>> = r
            .by_name
            .iter()
            .map(|n| {
                let s = n.stats.as_ref();
                vec![
                    n.name.clone(),
                    n.requests.to_string(),
                    (n.errors + n.http_errors).to_string(),
                    s.map(|s| fmt_ms(s.p50)).unwrap_or_else(|| "-".into()),
                    s.map(|s| fmt_ms(s.p90)).unwrap_or_else(|| "-".into()),
                    s.map(|s| fmt_ms(s.p99)).unwrap_or_else(|| "-".into()),
                    s.map(|s| fmt_ms(s.mean)).unwrap_or_else(|| "-".into()),
                ]
            })
            .collect();
        println!();
        out::table(&["REQUEST", "ANSWERED", "FAILED", "P50", "P90", "P99", "MEAN"], &rows);
    }
    if !r.histogram.is_empty() {
        println!("\n{}", out::bold("DISTRIBUTION (ms)"));
        let max = r.histogram.iter().map(|b| b.count).max().unwrap_or(1).max(1);
        let labels: Vec<String> = r
            .histogram
            .iter()
            .map(|b| match (b.from_ms, b.to_ms) {
                (_, None) => format!("> {}", b.from_ms),
                (0, Some(t)) => format!("<= {t}"),
                (f, Some(t)) => format!("{f} - {t}"),
            })
            .collect();
        let width = labels.iter().map(|l| l.len()).max().unwrap_or(0);
        for (b, label) in r.histogram.iter().zip(&labels) {
            let bar = "#".repeat(((b.count * 40) as f64 / max as f64).ceil() as usize);
            println!("  {label:>width$}  {} {}", out::cyan(&bar), b.count);
        }
    }
    let codes: Vec<String> = r.status_counts.iter().map(|(code, n)| format!("{code} × {n}")).collect();
    println!(
        "\n{} requests in {} · {:.2} req/s · {} received · {} failed to connect · {} answered with HTTP 4xx/5xx ({:.2}% errors)",
        r.total_requests,
        human_time(r.duration_ms),
        r.rps(),
        human_size(r.bytes),
        r.errors,
        r.http_errors,
        r.error_rate
    );
    if !codes.is_empty() {
        println!("status codes: {}", codes.join(", "));
    }
    if r.warmup_requests > 0 {
        println!("{}", out::dim(&format!("{} warm-up request(s) were sent first and are not included", r.warmup_requests)));
    }
    if let Some(a) = &r.aborted {
        println!("{}", out::yellow(a));
    }
    if !r.thresholds.is_empty() {
        println!("\n{}", out::bold("THRESHOLDS"));
        for t in &r.thresholds {
            let actual = match (&t.actual, &t.note) {
                (Some(a), _) => format!("measured {a:.2}"),
                (None, Some(n)) => n.clone(),
                _ => String::new(),
            };
            println!("  {} {}  {}", if t.passed { out::green("✓") } else { out::red("✗") }, t.expr, out::dim(&actual));
        }
    }
    if let Some(c) = comparison {
        println!("\n{}", out::bold(&format!("COMPARED WITH THE BASELINE (tolerance {}%)", c.tolerance_pct)));
        let rows: Vec<Vec<String>> = c
            .rows
            .iter()
            .map(|row| vec![row.metric.clone(), format!("{:.2}", row.baseline), format!("{:.2}", row.current), format!("{:+.1}%", row.change_pct), if row.regression { "REGRESSION".into() } else { "ok".into() }])
            .collect();
        out::table(&["METRIC", "BASELINE", "THIS RUN", "CHANGE", ""], &rows);
    }
}

fn print_profile_report(r: &rspostman_core::profile::ProfileReport, verbose: bool) {
    use rspostman_core::http::human_time;
    let has_usage = r.samples.iter().any(|s| s.usage.is_some());
    for (i, s) in r.samples.iter().enumerate() {
        let tag = out::dim(&format!("run {}", i + 1));
        match &s.error {
            Some(e) => println!("  {tag}  {} {e}", out::red("ERROR")),
            None => {
                let units = if has_usage {
                    format!("{} completion tokens", s.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0))
                } else {
                    format!("{} events", s.events)
                };
                let unit = if has_usage { "tok/s" } else { "evt/s" };
                println!(
                    "  {tag}  {}  ttft {}  total {}  {}  {:.1} {}  {}",
                    out::status(s.status, &s.status_text),
                    human_time(s.ttfb_ms),
                    human_time(s.total_ms),
                    units,
                    s.rate_per_s,
                    unit,
                    out::dim(s.model.as_deref().unwrap_or(""))
                );
            }
        }
    }
    let verdict = match r.verdict.as_str() {
        "healthy" => out::green("healthy"),
        "degraded" => out::yellow("degraded"),
        other => out::red(other),
    };
    let ok = r.samples.iter().filter(|s| s.error.is_none()).count();
    println!("\n{} {verdict} ({}/{})", out::bold("Verdict:"), ok, r.samples.len());
    for n in &r.notes {
        println!("  {} {n}", out::dim("note:"));
    }
    let rows: Vec<Vec<String>> = [("TTFT (ms)", &r.ttfb), ("Total (ms)", &r.total), (if has_usage { "Rate (tok/s)" } else { "Rate (evt/s)" }, &r.rate)]
        .iter()
        .filter_map(|(label, s)| {
            s.as_ref().map(|s| {
                vec![label.to_string(), fmt_ms(s.min), fmt_ms(s.p50), fmt_ms(s.p90), fmt_ms(s.p99), fmt_ms(s.max), fmt_ms(s.mean)]
            })
        })
        .collect();
    if !rows.is_empty() {
        out::table(&["", "MIN", "P50", "P90", "P99", "MAX", "MEAN"], &rows);
    }
    if verbose {
        for (i, s) in r.samples.iter().enumerate() {
            if !s.completion.is_empty() {
                println!("\n{} {}", out::dim(&format!("run {} output:", i + 1)), s.completion);
            }
        }
    }
}

fn resolve_item<'a>(c: &'a Collection, reference: &str) -> Result<&'a Item> {
    c.resolve(reference).ok_or_else(|| anyhow!("`{reference}` not found in collection `{}`", c.info.name))
}

fn stored(ws: &Workspace, reference: &str) -> Result<Collection> {
    let c = ws.find_collection(reference)?;
    if ws.collection(c.id())?.is_none() {
        bail!("`{reference}` is a file, not a workspace collection; run `rspostman import {reference}` first");
    }
    Ok(c)
}

fn write_out(text: &str, output: &Option<PathBuf>) -> Result<()> {
    match output {
        Some(p) => {
            std::fs::write(p, text).with_context(|| format!("cannot write {}", p.display()))?;
            eprintln!("written to {}", p.display());
        }
        None => println!("{text}"),
    }
    Ok(())
}

fn print_tree(items: &[Item], depth: usize) {
    for it in items {
        let pad = "  ".repeat(depth);
        match (&it.item, &it.request) {
            (Some(children), _) => {
                println!("{pad}{} {}", out::bold("▸"), out::bold(&it.name));
                print_tree(children, depth + 1);
            }
            (None, Some(r)) => println!("{pad}{} {}  {}", out::method(&r.method), it.name, out::dim(&r.url.raw)),
            _ => println!("{pad}{}", it.name),
        }
    }
}

const MASK: &str = "********";

/// Rows for `print_vars`, hiding the values of secret variables unless asked not to.
fn var_rows(env: &Environment, show_secrets: bool) -> Vec<(String, String, bool)> {
    env.values
        .iter()
        .map(|v| {
            let secret = v.kind.as_deref() == Some("secret");
            (v.key.clone(), if secret && !show_secrets && !v.value.is_empty() { MASK.to_string() } else { v.value.clone() }, v.enabled)
        })
        .collect()
}

/// Set a variable, asking for the value when it was not given on the command line.
fn set_var(env: &mut Environment, key: &str, value: Option<String>, secret: bool) -> Result<()> {
    let value = match value {
        Some(v) => v,
        None => prompt_secret(&format!("value for {key}"))?,
    };
    env.set(key, &value);
    if let Some(v) = env.values.iter_mut().find(|v| v.key == key) {
        // an existing secret marker is kept; it is only removed explicitly in the GUI
        if secret {
            v.kind = Some("secret".into());
        }
    }
    Ok(())
}

fn print_vars(rows: Vec<(String, String, bool)>) {
    if rows.is_empty() {
        println!("(no variables)");
        return;
    }
    out::table(
        &["KEY", "VALUE", ""],
        &rows.into_iter().map(|(k, v, on)| vec![k, v, if on { String::new() } else { "(disabled)".into() }]).collect::<Vec<_>>(),
    );
}

fn collection_cmd(ws: &Workspace, cmd: CollectionCmd) -> Result<i32> {
    match cmd {
        CollectionCmd::List => {
            let rows: Vec<Vec<String>> =
                ws.collections()?.iter().map(|c| vec![c.info.name.clone(), c.requests().len().to_string(), c.id().to_string()]).collect();
            if rows.is_empty() {
                println!("No collections yet. Create one with `rspostman collection create <name>` or `rspostman import <file>`.");
            } else {
                out::table(&["NAME", "REQUESTS", "ID"], &rows);
            }
        }
        CollectionCmd::Create { name, description } => {
            let mut c = Collection::new(&name);
            c.info.description = description;
            ws.save_collection(&c)?;
            println!("created collection `{name}` ({})", c.id());
        }
        CollectionCmd::Show { collection } => {
            let c = ws.find_collection(&collection)?;
            println!("{}  {}", out::bold(&c.info.name), out::dim(c.id()));
            if let Some(d) = &c.info.description {
                println!("{d}");
            }
            if let Some(a) = &c.auth {
                println!("auth: {}", a.kind);
            }
            if !c.variable.is_empty() {
                let shown = |v: &KV| if v.kind.as_deref() == Some("secret") && !v.value.is_empty() { MASK.to_string() } else { v.value.clone() };
                println!("variables: {}", c.variable.iter().map(|v| format!("{}={}", v.key, shown(v))).collect::<Vec<_>>().join(", "));
            }
            println!();
            print_tree(&c.item, 0);
        }
        CollectionCmd::Delete { collection } => {
            let c = stored(ws, &collection)?;
            ws.delete_collection(c.id())?;
            println!("deleted collection `{}`", c.info.name);
        }
        CollectionCmd::Rename { collection, name } => {
            let mut c = stored(ws, &collection)?;
            c.info.name = name.clone();
            ws.save_collection(&c)?;
            println!("renamed to `{name}`");
        }
        CollectionCmd::Duplicate { collection, name } => {
            let mut c = ws.find_collection(&collection)?;
            c.info.name = name.unwrap_or_else(|| format!("{} Copy", c.info.name));
            c.info.id = rspostman_core::model::new_id();
            ws.save_collection(&c)?;
            println!("created `{}` ({})", c.info.name, c.id());
        }
        CollectionCmd::AddFolder { collection, name, parent } => {
            let mut c = stored(ws, &collection)?;
            let pid = match &parent {
                Some(p) => Some(resolve_item(&c, p)?.id.clone()),
                None => None,
            };
            c.add_item(pid.as_deref(), Item::folder(&name))?;
            ws.save_collection(&c)?;
            println!("added folder `{name}`");
        }
        CollectionCmd::AddRequest { collection, name, target, parent, req } => {
            let mut c = stored(ws, &collection)?;
            let pid = match &parent {
                Some(p) => Some(resolve_item(&c, p)?.id.clone()),
                None => None,
            };
            let request = build_request(&target, &req)?;
            c.add_item(pid.as_deref(), Item::request(&name, request))?;
            ws.save_collection(&c)?;
            println!("added request `{name}`");
        }
        CollectionCmd::Remove { collection, item } => {
            let mut c = stored(ws, &collection)?;
            let id = resolve_item(&c, &item)?.id.clone();
            c.remove_item(&id);
            ws.save_collection(&c)?;
            println!("removed `{item}`");
        }
        CollectionCmd::Move { collection, item, to, index } => {
            let mut c = stored(ws, &collection)?;
            let id = resolve_item(&c, &item)?.id.clone();
            let pid = match &to {
                Some(p) => Some(resolve_item(&c, p)?.id.clone()),
                None => None,
            };
            c.move_item(&id, pid.as_deref(), index)?;
            ws.save_collection(&c)?;
            println!("moved `{item}`");
        }
        CollectionCmd::RenameItem { collection, item, name } => {
            let mut c = stored(ws, &collection)?;
            let id = resolve_item(&c, &item)?.id.clone();
            c.find_mut(&id).expect("resolved").name = name.clone();
            ws.save_collection(&c)?;
            println!("renamed to `{name}`");
        }
        CollectionCmd::SetVar { collection, key, value, secret } => {
            let mut c = stored(ws, &collection)?;
            let value = match value {
                Some(v) => v,
                None => prompt_secret(&format!("value for {key}"))?,
            };
            let mut vars = c.variables();
            vars.insert(key.clone(), value);
            c.set_variables(&vars);
            if secret {
                if let Some(v) = c.variable.iter_mut().find(|v| v.key == key) {
                    v.kind = Some("secret".into());
                }
            }
            ws.save_collection(&c)?;
            println!("set {key}");
        }
        CollectionCmd::UnsetVar { collection, key } => {
            let mut c = stored(ws, &collection)?;
            c.variable.retain(|v| v.key != key);
            ws.save_collection(&c)?;
            println!("unset {key}");
        }
        CollectionCmd::SetAuth { collection, item, kind, params } => {
            let mut c = stored(ws, &collection)?;
            let auth = build_auth(&kind, &params)?;
            match item {
                None => c.auth = auth,
                Some(i) => {
                    let id = resolve_item(&c, &i)?.id.clone();
                    let it = c.find_mut(&id).expect("resolved");
                    match it.request.as_mut() {
                        Some(r) => r.auth = auth,
                        None => it.auth = auth,
                    }
                }
            }
            ws.save_collection(&c)?;
            println!("auth set to {kind}");
        }
    }
    Ok(0)
}

fn stored_env(ws: &Workspace, reference: &str) -> Result<Environment> {
    let e = ws.find_environment(reference)?;
    if ws.environment(&e.id)?.is_none() {
        bail!("`{reference}` is a file, not a workspace environment; run `rspostman import {reference}` first");
    }
    Ok(e)
}

fn env_cmd(ws: &Workspace, cmd: EnvCmd) -> Result<i32> {
    match cmd {
        EnvCmd::List => {
            let active = ws.state()?.active_environment;
            let rows: Vec<Vec<String>> = ws
                .environments()?
                .iter()
                .map(|e| vec![if active.as_deref() == Some(&e.id) { "*".into() } else { String::new() }, e.name.clone(), e.values.len().to_string(), e.id.clone()])
                .collect();
            if rows.is_empty() {
                println!("No environments yet. Create one with `rspostman env create <name>`.");
            } else {
                out::table(&["", "NAME", "VARS", "ID"], &rows);
            }
        }
        EnvCmd::Create { name } => {
            let e = Environment::new(&name);
            ws.save_environment(&e)?;
            println!("created environment `{name}` ({})", e.id);
        }
        EnvCmd::Show { env, show_secrets } => {
            let e = ws.find_environment(&env)?;
            println!("{}  {}", out::bold(&e.name), out::dim(&e.id));
            print_vars(var_rows(&e, show_secrets));
        }
        EnvCmd::Delete { env } => {
            let e = stored_env(ws, &env)?;
            ws.delete_environment(&e.id)?;
            println!("deleted environment `{}`", e.name);
        }
        EnvCmd::Rename { env, name } => {
            let mut e = stored_env(ws, &env)?;
            e.name = name.clone();
            ws.save_environment(&e)?;
            println!("renamed to `{name}`");
        }
        EnvCmd::Set { env, key, value, secret } => {
            let mut e = stored_env(ws, &env)?;
            set_var(&mut e, &key, value, secret)?;
            ws.save_environment(&e)?;
            println!("set {key}");
        }
        EnvCmd::Unset { env, key } => {
            let mut e = stored_env(ws, &env)?;
            if !e.unset(&key) {
                bail!("variable `{key}` not found");
            }
            ws.save_environment(&e)?;
            println!("unset {key}");
        }
        EnvCmd::Use { env } => {
            let mut st = ws.state()?;
            if env == "none" {
                st.active_environment = None;
                println!("no active environment");
            } else {
                let e = stored_env(ws, &env)?;
                println!("active environment: {}", e.name);
                st.active_environment = Some(e.id);
            }
            ws.save_state(&st)?;
        }
    }
    Ok(0)
}

fn settings_cmd(ws: &Workspace, cmd: SettingsCmd) -> Result<i32> {
    match cmd {
        SettingsCmd::Show => println!("{}", serde_json::to_string_pretty(&ws.settings()?)?),
        SettingsCmd::Reset => {
            ws.save_settings(&Settings::default())?;
            println!("settings reset");
        }
        SettingsCmd::Set { key, value } => {
            let mut v = serde_json::to_value(ws.settings()?)?;
            let slot = v.get_mut(&key).ok_or_else(|| anyhow!("unknown setting `{key}`; see `rspostman settings show`"))?;
            *slot = match slot {
                serde_json::Value::Bool(_) => serde_json::Value::Bool(match value.as_str() {
                    "true" | "on" | "yes" | "1" => true,
                    "false" | "off" | "no" | "0" => false,
                    _ => bail!("`{key}` expects true or false"),
                }),
                serde_json::Value::Number(_) => serde_json::json!(value.parse::<u64>().map_err(|_| anyhow!("`{key}` expects a number"))?),
                _ => serde_json::Value::String(value),
            };
            ws.save_settings(&serde_json::from_value(v)?)?;
            println!("{key} updated");
        }
    }
    Ok(0)
}

fn import_cmd(ws: &Workspace, source: String, collection: Option<String>, name: Option<String>) -> Result<i32> {
    let text = if source == "-" {
        std::io::read_to_string(std::io::stdin())?
    } else if std::path::Path::new(&source).is_file() {
        std::fs::read_to_string(&source).with_context(|| format!("cannot read {source}"))?
    } else if source.trim_start().starts_with("curl") {
        source.clone()
    } else {
        bail!("`{source}` is neither a file nor a curl command");
    };
    match rspostman_core::import::parse(&text)? {
        Imported::Collection(c) => {
            let c = ws.import_collection(c)?;
            println!("imported collection `{}` with {} requests ({})", c.info.name, c.requests().len(), c.id());
        }
        Imported::Environment(mut e) => {
            if ws.environment(&e.id)?.is_some() {
                e.id = rspostman_core::model::new_id();
            }
            ws.save_environment(&e)?;
            println!("imported environment `{}` with {} variables", e.name, e.values.len());
        }
        Imported::Globals(g) => {
            let mut cur = ws.globals()?;
            for v in g.values.iter().filter(|v| v.enabled) {
                cur.set(&v.key, &v.value);
            }
            ws.save_globals(&cur)?;
            println!("merged {} global variables", g.values.len());
        }
        Imported::Request(r) => {
            let target = collection.unwrap_or_else(|| "Imported".into());
            let mut c = match ws.find_collection(&target) {
                Ok(c) if ws.collection(c.id())?.is_some() => c,
                _ => Collection::new(&target),
            };
            let name = name.unwrap_or_else(|| format!("{} {}", r.method, r.url.base()));
            c.add_item(None, Item::request(&name, r))?;
            ws.save_collection(&c)?;
            println!("imported request `{name}` into collection `{}`", c.info.name);
        }
    }
    Ok(0)
}

fn run(cli: Cli) -> Result<i32> {
    let ws = Workspace::open(cli.workspace.clone())?;
    match cli.cmd {
        Cmd::Send { target, req, exec } => {
            let request = build_request(&target, &req)?;
            let mut s = load_session(&ws, None, exec.env.as_deref())?;
            apply_exec(&mut s, &exec)?;
            let ex = s.send("", &request, request.auth.as_ref());
            if !exec.no_history {
                engine::record(&ws, &s, &ex, Some(&request), None)?;
            }
            s.persist(&ws, None)?;
            report(&ex, &exec)
        }
        Cmd::Request { collection, request, exec } => {
            let mut c = ws.find_collection(&collection)?;
            let id = resolve_item(&c, &request)?.id.clone();
            let mut s = load_session(&ws, Some(&c), exec.env.as_deref())?;
            apply_exec(&mut s, &exec)?;
            let ex = s.send_item(&c, &id);
            if !exec.no_history {
                engine::record(&ws, &s, &ex, c.find(&id).and_then(|i| i.request.as_ref()), Some(c.id()))?;
            }
            s.persist(&ws, Some(&mut c))?;
            report(&ex, &exec)
        }
        Cmd::Collection(cmd) => collection_cmd(&ws, cmd),
        Cmd::Env(cmd) => env_cmd(&ws, cmd),
        Cmd::Globals(cmd) => {
            let mut g = ws.globals()?;
            match cmd {
                VarCmd::List { show_secrets } => print_vars(var_rows(&g, show_secrets)),
                VarCmd::Set { key, value, secret } => {
                    set_var(&mut g, &key, value, secret)?;
                    ws.save_globals(&g)?;
                    println!("set {key}");
                }
                VarCmd::Unset { key } => {
                    if !g.unset(&key) {
                        bail!("variable `{key}` not found");
                    }
                    ws.save_globals(&g)?;
                    println!("unset {key}");
                }
            }
            Ok(0)
        }
        Cmd::History(cmd) => match cmd {
            HistoryCmd::List { limit } => {
                let h = ws.history()?;
                let rows: Vec<Vec<String>> = h
                    .iter()
                    .rev()
                    .take(limit)
                    .enumerate()
                    .map(|(i, e)| {
                        let (m, u) = e.request.as_ref().map(|p| (p.method.clone(), p.url.clone())).unwrap_or_default();
                        let st = match (&e.response, &e.error) {
                            (Some(r), _) => r.status.to_string(),
                            (None, Some(_)) => "ERR".into(),
                            _ => "-".into(),
                        };
                        vec![(i + 1).to_string(), e.timestamp.with_timezone(&chrono_local()).format("%Y-%m-%d %H:%M:%S").to_string(), st, m, u]
                    })
                    .collect();
                if rows.is_empty() {
                    println!("History is empty.");
                } else {
                    out::table(&["#", "TIME", "STATUS", "METHOD", "URL"], &rows);
                }
                Ok(0)
            }
            HistoryCmd::Show { entry } => {
                let e = ws.history_entry(&entry)?;
                let ex = Exchange { id: e.id, item_id: e.item_id, name: e.name.unwrap_or_default(), request: e.request, response: e.response, error: e.error, ..Default::default() };
                report(&ex, &ExecArgs { verbose: true, ..Default::default() })
            }
            HistoryCmd::Replay { entry, exec } => {
                let e = ws.history_entry(&entry)?;
                let mut source = e.source.ok_or_else(|| anyhow!("history entry has no replayable request"))?;
                let collection = match &e.collection_id {
                    Some(id) => ws.collection(id)?,
                    None => None,
                };
                if rspostman_core::secrets::was_redacted(&source) {
                    // credentials are not kept in the history; a saved request still has them
                    let saved = collection.as_ref().zip(e.item_id.as_ref()).and_then(|(c, id)| c.find(id)).and_then(|i| i.request.clone());
                    match saved {
                        Some(r) => {
                            eprintln!("{}", out::dim("credentials are not stored in the history; replaying the saved request from the collection"));
                            source = r;
                        }
                        None => bail!(
                            "this entry was sent with credentials, which are not stored in the history.\n       Send the request again with its credentials, or keep them in future entries with\n       `rspostman settings set history_store_secrets true`."
                        ),
                    }
                }
                let mut s = load_session(&ws, collection.as_ref(), exec.env.as_deref())?;
                apply_exec(&mut s, &exec)?;
                let auth = match (&collection, &e.item_id) {
                    (Some(c), Some(id)) if source.auth.is_none() => c.effective_auth(id),
                    _ => source.auth.clone(),
                };
                let mut ex = s.send(e.name.as_deref().unwrap_or(""), &source, auth.as_ref());
                ex.item_id = e.item_id.clone();
                if !exec.no_history {
                    engine::record(&ws, &s, &ex, Some(&source), e.collection_id.as_deref())?;
                }
                s.persist(&ws, None)?;
                report(&ex, &exec)
            }
            HistoryCmd::Clear => {
                ws.clear_history()?;
                println!("history cleared");
                Ok(0)
            }
        },
        Cmd::Cookies(cmd) => {
            let mut jar = ws.cookies()?;
            match cmd {
                CookiesCmd::List => {
                    let rows: Vec<Vec<String>> = jar
                        .cookies
                        .iter()
                        .map(|c| {
                            vec![
                                c.domain.clone(),
                                c.path.clone(),
                                c.name.clone(),
                                c.value.clone(),
                                c.expires.map(|e| e.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "session".into()),
                            ]
                        })
                        .collect();
                    if rows.is_empty() {
                        println!("Cookie jar is empty.");
                    } else {
                        out::table(&["DOMAIN", "PATH", "NAME", "VALUE", "EXPIRES"], &rows);
                    }
                }
                CookiesCmd::Set { url, cookie } => {
                    let u = url::Url::parse(&url).map_err(|e| anyhow!("invalid URL `{url}`: {e}"))?;
                    let c = jar.store_header(&cookie, &u).ok_or_else(|| anyhow!("cannot parse cookie `{cookie}` (expected `name=value; attributes`)"))?;
                    ws.save_cookies(&jar)?;
                    println!("stored cookie {} for {}", c.name, c.domain);
                }
                CookiesCmd::Clear { domain, name } => {
                    let n = match domain {
                        Some(d) => jar.remove(&d, name.as_deref()),
                        None => std::mem::take(&mut jar.cookies).len(),
                    };
                    ws.save_cookies(&jar)?;
                    println!("removed {n} cookie(s)");
                }
            }
            Ok(0)
        }
        Cmd::Import { source, collection, name } => import_cmd(&ws, source, collection, name),
        Cmd::Export(cmd) => {
            match cmd {
                ExportCmd::Collection { collection, output } => write_out(&ws.find_collection(&collection)?.to_json(), &output)?,
                ExportCmd::Env { env, output } => write_out(&ws.find_environment(&env)?.to_json(), &output)?,
                ExportCmd::Globals { output } => write_out(&ws.globals()?.to_json(), &output)?,
            }
            Ok(0)
        }
        Cmd::Codegen { lang, list, collection, request, target, req, env } => {
            if list {
                out::table(&["ID", "LANGUAGE"], &codegen::LANGUAGES.iter().map(|(i, n)| vec![i.to_string(), n.to_string()]).collect::<Vec<_>>());
                return Ok(0);
            }
            let prepared = match (collection, request) {
                (Some(cr), Some(rr)) => {
                    let c = ws.find_collection(&cr)?;
                    let item = resolve_item(&c, &rr)?;
                    let r = item.request.as_ref().ok_or_else(|| anyhow!("`{rr}` is a folder"))?;
                    let s = load_session(&ws, Some(&c), env.as_deref())?;
                    rspostman_core::http::prepare(r, c.effective_auth(&item.id).as_ref(), &s.scopes)?
                }
                _ => {
                    if target.is_empty() {
                        bail!("give either --collection/--request or `[METHOD] URL`");
                    }
                    let r = build_request(&target, &req)?;
                    let s = load_session(&ws, None, env.as_deref())?;
                    rspostman_core::http::prepare(&r, r.auth.as_ref(), &s.scopes)?
                }
            };
            println!("{}", codegen::generate(&lang, &prepared)?);
            Ok(0)
        }
        Cmd::Run { collection, run, report_json, report_junit, report_html, quiet, verbose } => {
            use rspostman_core::runner::{self, RunEvent};
            let mut c = ws.find_collection(&collection)?;
            let (mut session, opts) = load_run(&ws, &c, &run)?;
            if !quiet {
                eprintln!("{}", out::bold(&c.info.name));
            }
            let summary = runner::run(&c, &mut session, &opts, &mut |ev| {
                if quiet {
                    return true;
                }
                match ev {
                    RunEvent::Start { .. } => {}
                    RunEvent::Iteration { index, total } => {
                        if total > 1 {
                            eprintln!("\n{}", out::dim(&format!("Iteration {}/{total}", index + 1)));
                        }
                    }
                    RunEvent::Request(x) => {
                        let ex = &x.exchange;
                        eprintln!("\n{} {}", out::bold("→"), out::bold(&x.path));
                        if let Some(p) = &ex.request {
                            let meta = match &ex.response {
                                Some(r) => format!(
                                    "[{}, {}, {}]",
                                    out::status(r.status, &r.status_text),
                                    rspostman_core::http::human_size(r.size),
                                    rspostman_core::http::human_time(r.time_ms)
                                ),
                                None => String::new(),
                            };
                            eprintln!("  {} {} {meta}", p.method, out::dim(&p.url));
                        }
                        if ex.skipped {
                            eprintln!("  {}", out::yellow("skipped by pm.execution.skipRequest()"));
                        }
                        if let Some(e) = &ex.error {
                            eprintln!("  {} {}", out::red("✗"), out::red(e));
                        }
                        print_script_results(ex, verbose);
                        if verbose {
                            if let Some(r) = &ex.response {
                                for l in out::body_text(r, false).lines().take(60) {
                                    eprintln!("  {}", out::dim(l));
                                }
                            }
                        }
                    }
                }
                true
            })?;
            if !run.no_persist {
                session.persist(&ws, Some(&mut c))?;
            }
            print_summary(&summary);
            for (path, text) in [
                (&report_json, rspostman_core::report::json(&summary)),
                (&report_junit, rspostman_core::report::junit(&summary)),
                (&report_html, rspostman_core::report::html(&summary)),
            ] {
                if let Some(p) = path {
                    std::fs::write(p, text).with_context(|| format!("cannot write {}", p.display()))?;
                    eprintln!("report written to {}", p.display());
                }
            }
            Ok(if summary.success() { 0 } else { 1 })
        }
        Cmd::Benchmark {
            collection, request, folder, target, req, env, iterations, duration, concurrency, rate, delay, warmup, no_keepalive, asserts, compare, tolerance, insecure, timeout, report_json, report_csv, report_html, quiet,
        } => {
            use rspostman_core::profile;
            let mut session = load_session(&ws, None, env.as_deref())?;
            let mut tasks: Vec<profile::BenchTask> = vec![];
            match &collection {
                Some(cr) => {
                    let c = ws.find_collection(cr)?;
                    session = load_session(&ws, Some(&c), env.as_deref())?;
                    let all = c.requests();
                    let list: Vec<(String, &Item)> = match (&request, &folder) {
                        (Some(r), _) => {
                            let it = resolve_item(&c, r)?;
                            let path = all.iter().find(|(_, i)| i.id == it.id).map(|(p, _)| p.clone()).unwrap_or_else(|| it.name.clone());
                            vec![(path, it)]
                        }
                        (None, Some(f)) => {
                            let it = resolve_item(&c, f)?;
                            c.requests_in(&it.id)
                        }
                        (None, None) => all,
                    };
                    if list.is_empty() {
                        bail!("no requests to benchmark in collection `{}`", c.info.name);
                    }
                    for (path, item) in &list {
                        let Some(r) = &item.request else { continue };
                        let auth = c.effective_auth(&item.id);
                        let prepared = rspostman_core::http::prepare(r, auth.as_ref(), &session.scopes)?;
                        tasks.push(profile::BenchTask { name: path.clone(), prepared });
                    }
                }
                None => {
                    if target.is_empty() {
                        bail!("give either a collection (with --request or --folder) or an ad-hoc `[METHOD] URL`");
                    }
                    let r = build_request(&target, &req)?;
                    let prepared = rspostman_core::http::prepare(&r, r.auth.as_ref(), &session.scopes)?;
                    tasks.push(profile::BenchTask { name: format!("{} {}", prepared.method, rspostman_core::secrets::redact_url(&prepared.url)), prepared });
                }
            }
            use rspostman_core::bench;
            if concurrency > bench::MAX_CONCURRENCY {
                eprintln!("{}", out::yellow(&format!("concurrency limited to {}", bench::MAX_CONCURRENCY)));
            }
            if insecure {
                session.settings.verify_ssl = false;
            }
            if let Some(t) = timeout {
                session.settings.timeout_ms = t;
            }
            // fail on bad input before any request is sent
            bench::validate_thresholds(&asserts)?;
            if let Some(r) = rate {
                if !(r.is_finite() && r > 0.0) {
                    bail!("--rate must be a positive number of requests per second");
                }
            }
            let baseline: Option<bench::BenchReport> = match &compare {
                Some(p) => Some(
                    serde_json::from_str(&std::fs::read_to_string(p).with_context(|| format!("cannot read baseline {}", p.display()))?)
                        .with_context(|| format!("{} is not a benchmark JSON report", p.display()))?,
                ),
                None => None,
            };
            let opts = bench::BenchOptions {
                iterations,
                duration_ms: match &duration {
                    Some(d) => Some(parse_duration_ms(d)?),
                    None => None,
                },
                concurrency,
                delay_ms: delay,
                rate,
                warmup,
                keep_alive: !no_keepalive,
                thresholds: asserts,
            };
            if !quiet {
                let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
                let shape = match opts.duration_ms {
                    Some(ms) => format!("for {}", rspostman_core::http::human_time(ms)),
                    None => format!("{iterations} iteration(s)"),
                };
                let rate_note = opts.rate.map(|r| format!(" · {r} req/s")).unwrap_or_default();
                eprintln!("{} {}", out::bold("Benchmark"), out::dim(&format!("{} · {shape} · {} worker(s){rate_note}", names.join(", "), concurrency.clamp(1, bench::MAX_CONCURRENCY))));
            }
            // short runs list every request; long ones report progress once a second
            let list_each = !quiet && opts.duration_ms.is_none() && tasks.len() * iterations <= 60;
            let count = std::sync::atomic::AtomicUsize::new(0);
            let failed = std::sync::atomic::AtomicUsize::new(0);
            let last = std::sync::Mutex::new(std::time::Instant::now());
            let on_sample = |s: &bench::BenchSample| {
                let n = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if s.failed() {
                    failed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                if quiet {
                    return;
                }
                if list_each {
                    match &s.error {
                        Some(e) => eprintln!("  {} {}  {}", out::red("✗"), out::bold(&s.name), out::red(e)),
                        None => eprintln!("  {} {}  {} {}", out::dim(&format!("#{n}")), out::bold(&s.name), out::status(s.status, ""), out::dim(&rspostman_core::http::human_time(s.time_ms))),
                    }
                } else if let Ok(mut at) = last.lock() {
                    if at.elapsed() >= std::time::Duration::from_secs(1) {
                        *at = std::time::Instant::now();
                        eprintln!("  {}", out::dim(&format!("{n} requests, {} failed", failed.load(std::sync::atomic::Ordering::SeqCst))));
                    }
                }
            };
            let report = bench::benchmark(&tasks, &session.settings, &opts, None, &on_sample);
            let comparison = baseline.as_ref().map(|b| bench::compare(b, &report, tolerance));
            print_bench_summary(&report, comparison.as_ref());
            for (path, text) in [
                (&report_json, serde_json::to_string_pretty(&report)?),
                (&report_csv, bench::csv(&report)),
                (&report_html, rspostman_core::bench_report::html(&report, comparison.as_ref())),
            ] {
                if let Some(p) = path {
                    std::fs::write(p, text).with_context(|| format!("cannot write {}", p.display()))?;
                    eprintln!("report written to {}", p.display());
                }
            }
            let regressed = comparison.as_ref().map(|c| c.regressed).unwrap_or(false);
            Ok(if report.success() && !regressed { 0 } else { 1 })
        }
        Cmd::Profile { target, req, model, prompt, max_tokens, iterations, delay, env, verbose, insecure, timeout, report_json } => {
            use rspostman_core::profile;
            let mut request = build_request(&target, &req)?;
            // the generated chat body implies POST when the method was not given explicitly
            if target.len() == 1 {
                request.method = "POST".into();
            }
            // an explicit body (--json, -d, …) is kept; otherwise a chat request is generated
            if request.body.is_none() {
                let mut body = serde_json::json!({
                    "model": model,
                    "messages": [{"role": "user", "content": prompt}],
                    "stream": true,
                });
                if let Some(n) = max_tokens {
                    body["max_tokens"] = serde_json::json!(n);
                }
                request.body = Some(Body::raw(&body.to_string(), "json"));
            } else if let Some(raw) = request.body.as_ref().and_then(|b| b.raw.clone()) {
                // make sure an explicit JSON body asks for streaming
                if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if let Some(o) = v.as_object_mut() {
                        o.entry("stream".to_string()).or_insert(serde_json::Value::Bool(true));
                        request.body = Some(Body::raw(&v.to_string(), "json"));
                    }
                }
            }
            let mut session = load_session(&ws, None, env.as_deref())?;
            if insecure {
                session.settings.verify_ssl = false;
            }
            if let Some(t) = timeout {
                session.settings.timeout_ms = t;
            }
            let prepared = rspostman_core::http::prepare(&request, request.auth.as_ref(), &session.scopes)?;
            eprintln!("{} {}", out::method(&prepared.method), rspostman_core::secrets::redact_url(&prepared.url));
            let report = profile::profile_llm(&prepared, &session.settings, iterations, delay, None);
            print_profile_report(&report, verbose);
            if let Some(p) = &report_json {
                std::fs::write(p, serde_json::to_string_pretty(&report)?).with_context(|| format!("cannot write {}", p.display()))?;
                eprintln!("report written to {}", p.display());
            }
            Ok(if report.healthy() { 0 } else { 1 })
        }
        Cmd::Mock { collection, port, host, env, delay, no_cors } => {
            let c = ws.find_collection(&collection)?;
            let session = load_session(&ws, Some(&c), env.as_deref())?;
            let (srv, routes) = rspostman_core::mock::start(
                &c,
                session.scopes,
                &host,
                port,
                rspostman_core::mock::MockOptions { delay_ms: delay, no_cors },
                Some(Box::new(|line| eprintln!("{line}"))),
            )?;
            eprintln!("mock server for `{}` listening on {}  (Ctrl-C to stop)", c.info.name, srv.url());
            for r in &routes {
                eprintln!("  {} {}  {}", out::method(&r.method), r.pattern(), out::dim(&format!("{} → {} ({})", r.request_name, r.example.code, r.example.name)));
            }
            srv.join();
            Ok(0)
        }
        Cmd::Monitor { collection, every, count, run } => {
            use std::io::Write;
            let interval = parse_interval(&every)?;
            let dir = ws.root.join("monitors");
            rspostman_core::store::create_private_dir(&dir)?;
            let mut failures = 0;
            let mut n = 0;
            loop {
                n += 1;
                let mut c = ws.find_collection(&collection)?;
                let (mut session, opts) = load_run(&ws, &c, &run)?;
                let summary = rspostman_core::runner::run(&c, &mut session, &opts, &mut |_| true)?;
                if !run.no_persist {
                    session.persist(&ws, Some(&mut c))?;
                }
                let st = &summary.stats;
                let ok = summary.success();
                if !ok {
                    failures += 1;
                }
                println!(
                    "{} {} run #{n}: {} requests ({} failed), {} assertions ({} failed), {}",
                    summary.started.with_timezone(&chrono_local()).format("%Y-%m-%d %H:%M:%S"),
                    if ok { out::green("PASS") } else { out::red("FAIL") },
                    st.requests,
                    st.requests_failed,
                    st.tests,
                    st.tests_failed,
                    rspostman_core::http::human_time(summary.duration_ms)
                );
                let log = dir.join(format!("{}.jsonl", c.id().chars().map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' { ch } else { '_' }).collect::<String>()));
                let line = serde_json::json!({"started": summary.started, "success": ok, "duration_ms": summary.duration_ms, "stats": summary.stats, "aborted": summary.aborted,
                    "failures": summary.executions.iter().filter(|x| x.exchange.failed()).map(|x| serde_json::json!({"request": x.path, "error": x.exchange.error, "tests": x.exchange.tests.iter().filter(|t| !t.passed).map(|t| &t.name).collect::<Vec<_>>()})).collect::<Vec<_>>()});
                writeln!(std::fs::OpenOptions::new().create(true).append(true).open(&log)?, "{line}")?;
                rspostman_core::store::make_private(&log);
                if count.map(|c| n >= c).unwrap_or(false) {
                    eprintln!("monitor log: {}", log.display());
                    break;
                }
                std::thread::sleep(std::time::Duration::from_secs(interval));
            }
            Ok(if failures > 0 { 1 } else { 0 })
        }
        Cmd::Ws { url, headers, messages, wait, env } => {
            let session = load_session(&ws, None, env.as_deref())?;
            let mut hs = vec![];
            for h in &headers {
                let (k, v) = kv(h, ':', "header")?;
                hs.push((session.scopes.resolve(&k), session.scopes.resolve(&v)));
            }
            let client = rspostman_core::ws::WsClient::connect(&session.scopes.resolve(&url), &hs)?;
            let mut seen = 0;
            let flush = |client: &rspostman_core::ws::WsClient, seen: &mut usize| {
                for e in client.events_since(*seen) {
                    *seen += 1;
                    let tag = match e.kind.as_str() {
                        "sent" => out::yellow("↑"),
                        "message" | "binary" => out::green("↓"),
                        "error" => out::red("!"),
                        _ => out::dim("·"),
                    };
                    match e.kind.as_str() {
                        "message" | "binary" => println!("{tag} {}", e.data),
                        _ => eprintln!("{tag} {} {}", out::dim(&e.kind), e.data),
                    }
                }
            };
            if !messages.is_empty() {
                for m in &messages {
                    client.send(&session.scopes.resolve(m))?;
                }
                let end = std::time::Instant::now() + std::time::Duration::from_millis(wait);
                while std::time::Instant::now() < end && client.is_open() {
                    flush(&client, &mut seen);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                client.close();
            } else {
                eprintln!("{}", out::dim("type a message and press Enter to send; Ctrl-D closes the connection"));
                let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
                std::thread::spawn(move || {
                    for line in std::io::stdin().lines() {
                        match line {
                            Ok(l) => {
                                if tx.send(Some(l)).is_err() {
                                    return;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = tx.send(None);
                });
                while client.is_open() {
                    match rx.try_recv() {
                        Ok(Some(l)) if !l.is_empty() => client.send(&l)?,
                        Ok(None) => client.close(),
                        _ => {}
                    }
                    flush(&client, &mut seen);
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while client.is_open() && std::time::Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            flush(&client, &mut seen);
            Ok(if client.events_since(0).iter().any(|e| e.kind == "error") { 1 } else { 0 })
        }
        Cmd::Docs { collection, format, output } => {
            let c = ws.find_collection(&collection)?;
            let text = match format.as_str() {
                "md" | "markdown" => rspostman_core::docs::markdown(&c),
                "html" => rspostman_core::docs::html(&c),
                other => bail!("unknown format `{other}` (use md or html)"),
            };
            write_out(text.trim_end(), &output)?;
            Ok(0)
        }
        Cmd::Gui { port, no_open } => {
            let srv = rspostman_server::start(ws.clone(), port)?;
            let url = srv.url();
            eprintln!("rsPostman is running at {url}  (Ctrl-C to stop)");
            eprintln!("workspace: {}", ws.root.display());
            if !no_open {
                let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(target_os = "windows") { "explorer" } else { "xdg-open" };
                if std::process::Command::new(opener).arg(&url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_err() {
                    eprintln!("could not open a browser automatically; open {url} manually");
                }
            }
            srv.join();
            Ok(0)
        }
        Cmd::Vault(cmd) => vault_cmd(&ws, cmd),
        Cmd::Settings(cmd) => settings_cmd(&ws, cmd),
        Cmd::Echo { port, host } => {
            let srv = rspostman_core::echo::start(&host, port)?;
            eprintln!("echo server listening on {}  (Ctrl-C to stop)", srv.url());
            srv.join();
            Ok(0)
        }
    }
}

fn chrono_local() -> chrono::Local {
    chrono::Local
}

fn main() {
    let cli = Cli::parse();
    out::init(cli.no_color);
    match run(cli) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("{} {e:#}", out::red("error:"));
            std::process::exit(1);
        }
    }
}

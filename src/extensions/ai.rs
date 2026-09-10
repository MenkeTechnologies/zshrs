//! The `ai` builtin — a language-model call as a shell primitive.
//!
//! Ported from `strykelang/strykelang/ai.rs` (the `ai`/`prompt` family):
//! config resolution, the four providers, cost/token accounting, the
//! response cache, mock mode, retry/backoff, and the SSE stream decoder
//! all follow that source. What changes here is only the FRONT: stryke
//! passes `StrykeValue` opt pairs and returns a value; a shell builtin
//! takes argv and either writes to stdout or assigns to a parameter.
//!
//! zshrs-original — C zsh has no counterpart, so this lives under
//! `src/extensions/` per `docs/PORT.md`.
//!
//! # Why a builtin and not a script
//!
//! `ai -v reply "..."` performs the request AND the assignment inside
//! the shell process. The `curl | jq` equivalent costs two forks, two
//! execs, a pipe, and a command substitution to get one string into one
//! parameter. That elimination is the whole reason this is a builtin.
//!
//! # Deliberate divergence from the stryke source
//!
//! Two values in `strykelang/strykelang/ai.rs` are stale and are NOT
//! ported verbatim:
//!
//!   * the default model (`ai.rs:66`, `claude-opus-4-5`) — here
//!     `claude-opus-5`, via `config::AiConfig::default`;
//!   * the price table (`ai.rs:141-157`), which still carries
//!     Claude 3-era rates — see [`price_per_mtok`].
//!
//! Everything else is a faithful port.

use crate::config::AiConfig;
use parking_lot::Mutex;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// Failure of an `ai` invocation, split by what the caller should do
/// about it. The two arms map to distinct exit statuses.
#[derive(Debug)]
pub enum AiError {
    /// Bad flags / missing prompt — the user typed something wrong.
    /// Exit status 2.
    Usage(String),
    /// The call was well-formed but did not produce an answer: no API
    /// key, HTTP error, transport failure, cost ceiling. Exit status 1.
    Failed(String),
}

impl AiError {
    /// The exit status this failure maps to.
    pub fn status(&self) -> i32 {
        match self {
            AiError::Usage(_) => 2,
            AiError::Failed(_) => 1,
        }
    }

    /// The bare reason, for a `zshrs: ai: <reason>` stderr line.
    pub fn reason(&self) -> &str {
        match self {
            AiError::Usage(m) | AiError::Failed(m) => m,
        }
    }
}

type Result<T> = std::result::Result<T, AiError>;

fn usage<T>(msg: impl Into<String>) -> Result<T> {
    Err(AiError::Usage(msg.into()))
}

fn failed<T>(msg: impl Into<String>) -> Result<T> {
    Err(AiError::Failed(msg.into()))
}

/// What a successful `ai` run produced, and where it has to go.
///
/// `Printed` already went to stdout (possibly a token at a time, as it
/// arrived). The other two carry text the caller must install into a
/// shell parameter — [`crate::vm_helper`] owns the parameter table, so
/// this module hands the value back rather than reaching into it.
#[derive(Debug)]
pub enum Output {
    /// Already written to stdout; nothing left to do.
    Printed,
    /// Assign to the named scalar parameter.
    Scalar(String, String),
    /// Assign to the named array parameter, one element per line.
    Array(String, Vec<String>),
}

// ── Runtime configuration ──────────────────────────────────────────────
//
// `config::current()` is a process-wide cached snapshot of
// `~/.zshrs/zshrs.toml` and is deliberately never re-read. `ai -S` has
// to be able to change a default for the rest of the session, so the
// live values sit in an overlay seeded from that snapshot.

static RUNTIME: OnceLock<Mutex<AiConfig>> = OnceLock::new();

fn runtime() -> &'static Mutex<AiConfig> {
    RUNTIME.get_or_init(|| Mutex::new(crate::config::current().ai.clone()))
}

/// Apply one `key=value` assignment to the session's `[ai]` defaults.
/// Unknown keys are an error rather than a silent no-op — a typo in
/// `ai -S modle=...` must not look like it worked.
fn config_set(assignment: &str) -> Result<()> {
    let Some((key, value)) = assignment.split_once('=') else {
        return usage(format!("-S wants key=value, got `{}`", assignment));
    };
    let mut cfg = runtime().lock();
    match key {
        "provider" => cfg.provider = value.to_string(),
        "model" => cfg.model = value.to_string(),
        "api_key_env" => cfg.api_key_env = value.to_string(),
        "base_url" => cfg.base_url = value.to_string(),
        "cache" => cfg.cache = matches!(value, "1" | "true" | "yes" | "on"),
        "max_cost_run" => match value.parse() {
            Ok(v) => cfg.max_cost_run = v,
            Err(_) => return usage(format!("max_cost_run wants a number, got `{}`", value)),
        },
        "max_tokens" => match value.parse() {
            Ok(v) => cfg.max_tokens = v,
            Err(_) => return usage(format!("max_tokens wants an integer, got `{}`", value)),
        },
        "timeout" => match value.parse() {
            Ok(v) => cfg.timeout = v,
            Err(_) => return usage(format!("timeout wants an integer, got `{}`", value)),
        },
        other => return usage(format!("unknown config key `{}`", other)),
    }
    Ok(())
}

/// Render the session's `[ai]` defaults as `key=value` lines.
fn config_dump() -> String {
    let cfg = runtime().lock();
    let mut out = String::new();
    out.push_str(&format!("provider={}\n", cfg.provider));
    out.push_str(&format!("model={}\n", cfg.model));
    out.push_str(&format!("api_key_env={}\n", cfg.api_key_env));
    out.push_str(&format!("base_url={}\n", cfg.base_url));
    out.push_str(&format!("cache={}\n", cfg.cache));
    out.push_str(&format!("max_cost_run={}\n", cfg.max_cost_run));
    out.push_str(&format!("max_tokens={}\n", cfg.max_tokens));
    out.push_str(&format!("timeout={}\n", cfg.timeout));
    out
}

// ── Cost tracking (stryke ai.rs:122-157) ───────────────────────────────

static COST_USD_MICROS: AtomicU64 = AtomicU64::new(0);
static INPUT_TOKENS: AtomicU64 = AtomicU64::new(0);
static OUTPUT_TOKENS: AtomicU64 = AtomicU64::new(0);
static CACHE_CREATION_TOKENS: AtomicU64 = AtomicU64::new(0);
static CACHE_READ_TOKENS: AtomicU64 = AtomicU64::new(0);

fn add_cost(usd: f64) {
    COST_USD_MICROS.fetch_add((usd * 1_000_000.0) as u64, Ordering::Relaxed);
}

fn current_cost_usd() -> f64 {
    COST_USD_MICROS.load(Ordering::Relaxed) as f64 / 1_000_000.0
}

/// Published per-million-token rates, `(input, output)` in USD.
///
/// Hardcoded so cost accounting never depends on a network call, and
/// keyed by prefix so a dated snapshot id resolves to its family. An
/// unrecognised model falls back to Sonnet-class rates, which makes an
/// unknown model over-report rather than under-report against the
/// `max_cost_run` ceiling.
///
/// This is the one place the port deviates in VALUE from
/// `strykelang/strykelang/ai.rs:141-157`: that table predates the
/// Claude 4/5 families and still prices `claude-opus` at $15/$75 per
/// MTok. Order matters — `claude-opus-4-5` must be tested before the
/// bare `claude-opus` prefix.
fn price_per_mtok(model: &str) -> (f64, f64) {
    match model {
        m if m.starts_with("claude-fable") || m.starts_with("claude-mythos") => (10.00, 50.00),
        m if m.starts_with("claude-opus-4-5") => (5.00, 25.00),
        m if m.starts_with("claude-opus") => (5.00, 25.00),
        m if m.starts_with("claude-sonnet-4-6") => (3.00, 15.00),
        m if m.starts_with("claude-sonnet") => (2.00, 10.00),
        m if m.starts_with("claude-haiku") => (1.00, 5.00),
        m if m.starts_with("gpt-4o-mini") => (0.15, 0.60),
        m if m.starts_with("gpt-4o") => (2.50, 10.00),
        m if m.starts_with("gpt-5") => (5.00, 20.00),
        m if m.starts_with("o1") => (15.00, 60.00),
        m if m.starts_with("gemini-2.5-pro") => (1.25, 10.00),
        m if m.starts_with("gemini") => (0.30, 2.50),
        _ => (3.00, 15.00),
    }
}

/// Bill one exchange against the running total.
///
/// The two cache figures are Anthropic-only and priced off the INPUT
/// rate: a cache write costs 1.25x base, a cache read 0.10x
/// (stryke ai.rs:1050-1053). Providers that report no cache usage pass
/// zeroes and the term vanishes.
fn bill(model: &str, input: u64, output: u64, cache_creation: u64, cache_read: u64) {
    INPUT_TOKENS.fetch_add(input, Ordering::Relaxed);
    OUTPUT_TOKENS.fetch_add(output, Ordering::Relaxed);
    CACHE_CREATION_TOKENS.fetch_add(cache_creation, Ordering::Relaxed);
    CACHE_READ_TOKENS.fetch_add(cache_read, Ordering::Relaxed);
    let (in_rate, out_rate) = price_per_mtok(model);
    let per_token_in = in_rate / 1_000_000.0;
    add_cost(
        input as f64 * per_token_in
            + output as f64 * (out_rate / 1_000_000.0)
            + cache_creation as f64 * per_token_in * 1.25
            + cache_read as f64 * per_token_in * 0.10,
    );
}

// ── Response cache (stryke ai.rs:159-181) ──────────────────────────────

static CACHE: OnceLock<Mutex<indexmap::IndexMap<String, String>>> = OnceLock::new();
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

fn cache() -> &'static Mutex<indexmap::IndexMap<String, String>> {
    CACHE.get_or_init(|| Mutex::new(indexmap::IndexMap::new()))
}

/// SHA-256 over the four inputs that determine a response, NUL-separated
/// so `("ab", "c")` and `("a", "bc")` cannot collide.
fn cache_key(provider: &str, model: &str, system: &str, prompt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for field in [provider, model, system, prompt] {
        h.update(field.as_bytes());
        h.update(b"\x00");
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

// ── Mock mode (stryke ai.rs:183-215) ───────────────────────────────────

static MOCKS: OnceLock<Mutex<Vec<(regex::Regex, String)>>> = OnceLock::new();

fn mocks() -> &'static Mutex<Vec<(regex::Regex, String)>> {
    MOCKS.get_or_init(|| Mutex::new(Vec::new()))
}

fn match_mock(prompt: &str) -> Option<String> {
    mocks()
        .lock()
        .iter()
        .find(|(re, _)| re.is_match(prompt))
        .map(|(_, response)| response.clone())
}

/// True when `$ZSHRS_AI_MODE` forbids reaching the network at all.
/// Tests set this so a missing mock is a hard error instead of a silent
/// live call that costs money and needs a key.
fn mock_only_mode() -> bool {
    matches!(
        std::env::var("ZSHRS_AI_MODE").as_deref(),
        Ok("mock-only") | Ok("mock_only")
    )
}

// ── Call history (stryke ai.rs:2548-2600) ──────────────────────────────

#[derive(Clone, Debug)]
struct HistoryEntry {
    provider: String,
    model: String,
    prompt: String,
    response_chars: usize,
    usd: f64,
    cache_hit: bool,
}

const HISTORY_CAP: usize = 100;
static HISTORY: OnceLock<Mutex<std::collections::VecDeque<HistoryEntry>>> = OnceLock::new();

fn history() -> &'static Mutex<std::collections::VecDeque<HistoryEntry>> {
    HISTORY.get_or_init(|| Mutex::new(std::collections::VecDeque::with_capacity(HISTORY_CAP)))
}

fn record_history(entry: HistoryEntry) {
    let mut g = history().lock();
    if g.len() >= HISTORY_CAP {
        g.pop_front();
    }
    g.push_back(entry);
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}

// ── Resolved per-call options ──────────────────────────────────────────

/// One invocation's settings, after flags have been merged over the
/// session defaults. Mirrors the `opt_str`/`opt_int`/`opt_bool` block at
/// the top of stryke's `ai_prompt` (`ai.rs:227-236`).
struct Call {
    provider: String,
    model: String,
    system: String,
    prompt: String,
    max_tokens: i64,
    /// Negative means "send no `temperature` field at all", which is how
    /// stryke distinguishes unset from an explicit `0.0`.
    temperature: f64,
    timeout: i64,
    cache: bool,
    cache_control: bool,
    stream: bool,
}

/// Where generated text goes as it arrives.
enum Sink<'a> {
    /// Straight to stdout, flushed per delta so a stream is visible as
    /// it is produced rather than at end of turn.
    Stdout(std::io::StdoutLock<'a>),
    /// Accumulated for assignment to a shell parameter.
    Buffer(String),
}

impl Sink<'_> {
    fn push(&mut self, chunk: &str) {
        match self {
            Sink::Stdout(out) => {
                let _ = out.write_all(chunk.as_bytes());
                let _ = out.flush();
            }
            Sink::Buffer(buf) => buf.push_str(chunk),
        }
    }
}

// ── Flag parsing ───────────────────────────────────────────────────────

/// Everything `parse_flags` extracted, before defaults are applied.
#[derive(Default, Debug)]
struct Flags {
    model: Option<String>,
    system: Option<String>,
    provider: Option<String>,
    max_tokens: Option<i64>,
    temperature: Option<f64>,
    timeout: Option<i64>,
    no_cache: bool,
    cache_control: bool,
    buffered: bool,
    var: Option<String>,
    array: Option<String>,
    words: Vec<String>,
    /// A report/maintenance flag short-circuits the whole call.
    report: Option<Report>,
    set: Vec<String>,
}

/// The non-calling modes. These are flags rather than subcommand words
/// on purpose: `ai cost of living in Oslo` has to stay a prompt.
#[derive(Debug)]
enum Report {
    Cost,
    ClearCache,
    History,
    Config,
    Mock(String),
    Help,
}

fn parse_i64(flag: &str, raw: Option<&String>) -> Result<i64> {
    match raw {
        Some(v) => v
            .parse()
            .map_err(|_| AiError::Usage(format!("{} wants an integer, got `{}`", flag, v))),
        None => usage(format!("{} requires an argument", flag)),
    }
}

fn take<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a String> {
    *i += 1;
    args.get(*i)
        .ok_or_else(|| AiError::Usage(format!("{} requires an argument", flag)))
}

fn parse_flags(args: &[String]) -> Result<Flags> {
    let mut f = Flags::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--" => {
                f.words.extend_from_slice(&args[i + 1..]);
                break;
            }
            "-m" => f.model = Some(take(args, &mut i, "-m")?.clone()),
            "-s" => f.system = Some(take(args, &mut i, "-s")?.clone()),
            "-P" => f.provider = Some(take(args, &mut i, "-P")?.clone()),
            "-v" => f.var = Some(take(args, &mut i, "-v")?.clone()),
            "-a" => f.array = Some(take(args, &mut i, "-a")?.clone()),
            "-M" => f.report = Some(Report::Mock(take(args, &mut i, "-M")?.clone())),
            "-S" => f.set.push(take(args, &mut i, "-S")?.clone()),
            "-t" => {
                let v = take(args, &mut i, "-t")?.clone();
                f.max_tokens = Some(parse_i64("-t", Some(&v))?);
            }
            "-o" => {
                let v = take(args, &mut i, "-o")?.clone();
                f.timeout = Some(parse_i64("-o", Some(&v))?);
            }
            "-T" => {
                let v = take(args, &mut i, "-T")?.clone();
                match v.parse::<f64>() {
                    Ok(t) => f.temperature = Some(t),
                    Err(_) => return usage(format!("-T wants a number, got `{}`", v)),
                }
            }
            "-n" => f.no_cache = true,
            "-k" => f.cache_control = true,
            "-b" => f.buffered = true,
            "-c" => f.report = Some(Report::Cost),
            "-K" => f.report = Some(Report::ClearCache),
            "-H" => f.report = Some(Report::History),
            "-G" => f.report = Some(Report::Config),
            "-h" | "--help" => f.report = Some(Report::Help),
            // A lone `-` is the conventional "read stdin" word, not a
            // flag; anything else starting with `-` is a typo worth
            // reporting rather than silently prompting with.
            "-" => f.words.push(arg.clone()),
            other if other.starts_with('-') && other.len() > 1 => {
                return usage(format!("bad option: {}", other));
            }
            _ => f.words.push(arg.clone()),
        }
        i += 1;
    }
    Ok(f)
}

const HELP: &str = "\
usage: ai [-m model] [-s system] [-P provider] [-t max-tokens] [-T temp]
          [-o timeout] [-n] [-k] [-b] [-v var | -a array] [--] prompt...
       ai -c | -K | -H | -G | -S key=value | -M pattern=response

  -m MODEL     model id                    -s SYS     system prompt
  -P PROVIDER  anthropic|openai|local|ollama|gemini
  -t N         max output tokens           -T FLOAT   temperature
  -o SECS      request timeout             -n         bypass the cache
  -k           cache the system prefix     -b         buffer, do not stream
  -v VAR       assign to scalar VAR        -a ARRAY   assign lines to ARRAY
  -c           cost and token report       -K         clear the cache
  -H           recent call history         -G         show config
  -S key=value set a session default       -M pat=rsp install a mock

With no prompt words, or a lone `-`, the prompt is read from stdin.
";

// ── Reports ────────────────────────────────────────────────────────────

fn cost_report() -> String {
    format!(
        "usd={:.6}\ninput_tokens={}\noutput_tokens={}\ncache_creation_tokens={}\n\
         cache_read_tokens={}\ncache_hits={}\ncache_misses={}\n",
        current_cost_usd(),
        INPUT_TOKENS.load(Ordering::Relaxed),
        OUTPUT_TOKENS.load(Ordering::Relaxed),
        CACHE_CREATION_TOKENS.load(Ordering::Relaxed),
        CACHE_READ_TOKENS.load(Ordering::Relaxed),
        CACHE_HITS.load(Ordering::Relaxed),
        CACHE_MISSES.load(Ordering::Relaxed),
    )
}

/// One call per line, tab-separated, so the output is `cut`-able and
/// `read -A`-able — the shell-native equivalent of stryke's `ai_history`
/// returning an arrayref of hashrefs (`ai.rs:2601-2634`). Tabs and
/// newlines inside the recorded prompt are flattened to spaces so a
/// multi-line prompt cannot forge extra columns or rows.
fn history_report() -> String {
    let g = history().lock();
    let mut out = String::new();
    for e in g.iter() {
        out.push_str(&format!(
            "{}\t{}\t{:.6}\t{}\t{}\t{}\n",
            e.provider,
            e.model,
            e.usd,
            e.response_chars,
            if e.cache_hit { "hit" } else { "miss" },
            e.prompt.replace(['\t', '\n'], " "),
        ));
    }
    out
}

// ── Entry point ────────────────────────────────────────────────────────

/// Run one `ai` invocation. `args` is argv WITHOUT the builtin name.
///
/// Streams to stdout unless the result is destined for a parameter, in
/// which case it is buffered and handed back for the caller to assign.
pub fn run(args: &[String]) -> Result<Output> {
    let flags = parse_flags(args)?;

    for assignment in &flags.set {
        config_set(assignment)?;
    }
    if let Some(report) = flags.report {
        return report_output(report);
    }
    // `-S` on its own is a configuration command, not a prompt.
    if !flags.set.is_empty() && flags.words.is_empty() {
        return Ok(Output::Printed);
    }
    if flags.var.is_some() && flags.array.is_some() {
        return usage("-v and -a are mutually exclusive");
    }

    let call = resolve(&flags)?;
    let want_buffer = flags.var.is_some() || flags.array.is_some() || flags.buffered;

    let stdout = std::io::stdout();
    let mut sink = if want_buffer {
        Sink::Buffer(String::new())
    } else {
        Sink::Stdout(stdout.lock())
    };
    let text = execute(&call, &mut sink)?;

    match (flags.var, flags.array) {
        (Some(var), _) => Ok(Output::Scalar(var, text)),
        (_, Some(arr)) => Ok(Output::Array(
            arr,
            text.lines().map(str::to_string).collect(),
        )),
        _ => {
            // Buffered-but-unassigned (`-b`) still has to reach stdout,
            // and a streamed response needs the trailing newline the
            // API never sends.
            let mut out = std::io::stdout().lock();
            if flags.buffered {
                let _ = out.write_all(text.as_bytes());
            }
            if !text.ends_with('\n') {
                let _ = out.write_all(b"\n");
            }
            let _ = out.flush();
            Ok(Output::Printed)
        }
    }
}

fn report_output(report: Report) -> Result<Output> {
    let text = match report {
        Report::Cost => cost_report(),
        Report::History => history_report(),
        Report::Config => config_dump(),
        Report::Help => HELP.to_string(),
        Report::ClearCache => {
            cache().lock().clear();
            String::new()
        }
        Report::Mock(spec) => {
            let Some((pattern, response)) = spec.split_once('=') else {
                return usage(format!("-M wants pattern=response, got `{}`", spec));
            };
            match regex::Regex::new(pattern) {
                Ok(re) => mocks().lock().push((re, response.to_string())),
                Err(e) => return usage(format!("-M bad pattern `{}`: {}", pattern, e)),
            }
            String::new()
        }
    };
    if !text.is_empty() {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
    Ok(Output::Printed)
}

/// Merge flags over the session defaults and pull the prompt together.
fn resolve(flags: &Flags) -> Result<Call> {
    let cfg = runtime().lock().clone();
    let prompt = if flags.words.is_empty() || flags.words == ["-"] {
        read_stdin_prompt()?
    } else {
        flags.words.join(" ")
    };
    if prompt.trim().is_empty() {
        return usage("empty prompt");
    }
    Ok(Call {
        provider: flags.provider.clone().unwrap_or(cfg.provider),
        model: flags.model.clone().unwrap_or(cfg.model),
        system: flags.system.clone().unwrap_or_default(),
        prompt,
        max_tokens: flags.max_tokens.unwrap_or(cfg.max_tokens),
        temperature: flags.temperature.unwrap_or(-1.0),
        timeout: flags.timeout.unwrap_or(cfg.timeout),
        cache: cfg.cache && !flags.no_cache,
        cache_control: flags.cache_control,
        // Streaming is Anthropic-only; the other providers here are
        // request/response, and a stream flag on them would be a lie.
        stream: !flags.buffered && flags.var.is_none() && flags.array.is_none(),
    })
}

fn read_stdin_prompt() -> Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    match std::io::stdin().read_to_string(&mut buf) {
        Ok(_) => Ok(buf),
        Err(e) => failed(format!("stdin: {}", e)),
    }
}

/// The dispatch order of stryke's `ai_prompt` (`ai.rs:238-395`), kept
/// intact: mock, then cache, then cost ceiling, then the provider.
fn execute(call: &Call, sink: &mut Sink) -> Result<String> {
    if let Some(response) = match_mock(&call.prompt) {
        sink.push(&response);
        record_history(HistoryEntry {
            provider: call.provider.clone(),
            model: call.model.clone(),
            prompt: truncate(&call.prompt, 200),
            response_chars: response.chars().count(),
            usd: 0.0,
            cache_hit: false,
        });
        return Ok(response);
    }
    if mock_only_mode() {
        return failed(format!(
            "ZSHRS_AI_MODE=mock-only and no mock matched prompt {:?}",
            truncate(&call.prompt, 60)
        ));
    }

    let key = cache_key(&call.provider, &call.model, &call.system, &call.prompt);
    if call.cache {
        let hit = cache().lock().get(&key).cloned();
        if let Some(hit) = hit {
            CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            sink.push(&hit);
            record_history(HistoryEntry {
                provider: call.provider.clone(),
                model: call.model.clone(),
                prompt: truncate(&call.prompt, 200),
                response_chars: hit.chars().count(),
                usd: 0.0,
                cache_hit: true,
            });
            return Ok(hit);
        }
        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    }

    let ceiling = runtime().lock().max_cost_run;
    if ceiling > 0.0 && current_cost_usd() >= ceiling {
        return failed(format!(
            "max_cost_run={:.2} exceeded (spent ${:.4} so far)",
            ceiling,
            current_cost_usd()
        ));
    }

    let before = current_cost_usd();
    let text = match call.provider.as_str() {
        "anthropic" => {
            if call.stream {
                call_anthropic_stream(call, sink)?
            } else {
                let t = call_anthropic(call)?;
                sink.push(&t);
                t
            }
        }
        "openai" => {
            let t = call_openai(
                call,
                "https://api.openai.com/v1/chat/completions",
                "OPENAI_API_KEY",
            )?;
            sink.push(&t);
            t
        }
        "openai_compat" | "compat" | "local" => {
            let base = base_url_or(
                call,
                "ZSHRS_AI_BASE_URL",
                "http://localhost:1234/v1/chat/completions",
            );
            let t = call_openai(call, &base, "ZSHRS_AI_LOCAL_KEY")?;
            sink.push(&t);
            t
        }
        "ollama" => {
            let t = call_ollama(call)?;
            sink.push(&t);
            t
        }
        "gemini" | "google" => {
            let t = call_gemini(call)?;
            sink.push(&t);
            t
        }
        other => {
            return usage(format!(
                "unknown provider `{}` (anthropic, openai, local, ollama, gemini)",
                other
            ));
        }
    };

    if call.cache {
        cache().lock().insert(key, text.clone());
    }
    record_history(HistoryEntry {
        provider: call.provider.clone(),
        model: call.model.clone(),
        prompt: truncate(&call.prompt, 200),
        response_chars: text.chars().count(),
        usd: current_cost_usd() - before,
        cache_hit: false,
    });
    Ok(text)
}

/// Config `base_url`, else the named env var, else the built-in default.
fn base_url_or(call: &Call, env: &str, fallback: &str) -> String {
    let configured = runtime().lock().base_url.clone();
    if !configured.is_empty() {
        return configured;
    }
    let _ = call;
    std::env::var(env).unwrap_or_else(|_| fallback.to_string())
}

fn agent(timeout: i64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout.max(1) as u64))
        .build()
}

// ── Provider: Anthropic (stryke ai.rs:994-1106) ────────────────────────

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

fn anthropic_key(call: &Call) -> Result<String> {
    let env = runtime().lock().api_key_env.clone();
    let _ = call;
    std::env::var(&env).map_err(|_| AiError::Failed(format!("${} is not set", env)))
}

/// Build the request body shared by the buffered and streaming paths.
fn anthropic_body(call: &Call, stream: bool) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": call.model,
        "max_tokens": call.max_tokens,
        "messages": [{ "role": "user", "content": call.prompt }],
    });
    if !call.system.is_empty() {
        body["system"] = if call.cache_control {
            // Turning the system prefix into a block list with
            // `cache_control` is what makes a repeated system prompt
            // bill at cache-read rates instead of full input rates.
            serde_json::json!([{
                "type": "text",
                "text": call.system,
                "cache_control": { "type": "ephemeral" },
            }])
        } else {
            serde_json::Value::String(call.system.clone())
        };
    }
    if call.temperature >= 0.0 {
        body["temperature"] = serde_json::Value::from(call.temperature);
    }
    if stream {
        body["stream"] = serde_json::Value::Bool(true);
    }
    body
}

fn should_retry(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Exponential backoff, capped: 1s, 2s, 4s, 8s, 16s, 30s.
fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.min(5)).min(30))
}

const MAX_ATTEMPTS: u32 = 4;

fn call_anthropic(call: &Call) -> Result<String> {
    let key = anthropic_key(call)?;
    let agent = agent(call.timeout);
    let body = anthropic_body(call, false);

    let mut last: Option<AiError> = None;
    let mut json = None;
    for attempt in 0..MAX_ATTEMPTS {
        match agent
            .post(ANTHROPIC_URL)
            .set("x-api-key", &key)
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("content-type", "application/json")
            .send_json(body.clone())
        {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(v) => {
                    json = Some(v);
                    break;
                }
                Err(e) => return failed(format!("anthropic decode: {}", e)),
            },
            Err(ureq::Error::Status(code, resp)) => {
                if attempt + 1 < MAX_ATTEMPTS && should_retry(code) {
                    std::thread::sleep(retry_delay(attempt));
                    continue;
                }
                let text = resp.into_string().unwrap_or_default();
                last = Some(AiError::Failed(format!(
                    "anthropic {}: {}",
                    code,
                    truncate(&text, 200)
                )));
                break;
            }
            Err(ureq::Error::Transport(t)) => {
                if attempt + 1 < MAX_ATTEMPTS {
                    std::thread::sleep(retry_delay(attempt));
                    continue;
                }
                last = Some(AiError::Failed(format!("anthropic transport: {}", t)));
                break;
            }
        }
    }
    let Some(json) = json else {
        return Err(last.unwrap_or_else(|| AiError::Failed("anthropic call failed".into())));
    };

    let usage = &json["usage"];
    bill(
        &call.model,
        usage["input_tokens"].as_u64().unwrap_or(0),
        usage["output_tokens"].as_u64().unwrap_or(0),
        usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
    );

    let mut out = String::new();
    if let Some(blocks) = json["content"].as_array() {
        for block in blocks {
            if block["type"].as_str() == Some("text") {
                if let Some(t) = block["text"].as_str() {
                    out.push_str(t);
                }
            }
        }
    }
    if out.is_empty() {
        // A refusal is a 200 with no text block; say which, because
        // "no content" alone sends the user hunting for a network fault
        // that isn't there.
        if let Some(reason) = json["stop_reason"].as_str() {
            if reason == "refusal" {
                return failed("anthropic declined the request (stop_reason=refusal)");
            }
        }
        return failed(format!(
            "anthropic returned no content: {}",
            truncate(&json.to_string(), 200)
        ));
    }
    Ok(out)
}

/// Streaming variant. Deltas reach `sink` as they arrive; the assembled
/// text still comes back so the cache and history see the same value the
/// buffered path would have produced.
fn call_anthropic_stream(call: &Call, sink: &mut Sink) -> Result<String> {
    let key = anthropic_key(call)?;
    let resp = agent(call.timeout)
        .post(ANTHROPIC_URL)
        .set("x-api-key", &key)
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("content-type", "application/json")
        .set("accept", "text/event-stream")
        .send_json(anthropic_body(call, true));
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            return failed(format!("anthropic {}: {}", code, truncate(&text, 200)));
        }
        Err(ureq::Error::Transport(t)) => {
            return failed(format!("anthropic transport: {}", t));
        }
    };
    let reader = std::io::BufReader::new(resp.into_reader());
    decode_sse(&call.model, reader, sink)
}

/// Decode an Anthropic SSE body, pushing each text delta to `sink`.
///
/// Ported from stryke's `AnthropicStreamIter::next_item`
/// (`ai.rs:482-541`) with one correction the shape makes possible:
/// there, the token counters are locals inside `next_item`, so they are
/// reset on every call and only whatever arrived during the LAST call is
/// ever billed. Draining the stream in one loop lets the counters live
/// across the whole response, which is what the accounting meant.
fn decode_sse<R: std::io::BufRead>(model: &str, mut reader: R, sink: &mut Sink) -> Result<String> {
    let mut text = String::new();
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    let mut cache_creation = 0u64;
    let mut cache_read = 0u64;
    let mut error: Option<String> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                error = Some(format!("anthropic stream: {}", e));
                break;
            }
        }
        let Some(payload) = line.trim_end().strip_prefix("data: ") else {
            continue;
        };
        if payload == "[DONE]" {
            break;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        match event["type"].as_str() {
            Some("content_block_delta") => {
                if let Some(t) = event["delta"]["text"].as_str() {
                    sink.push(t);
                    text.push_str(t);
                }
            }
            Some("message_start") => {
                let usage = &event["message"]["usage"];
                input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                cache_creation = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            }
            Some("message_delta") => {
                output_tokens = event["usage"]["output_tokens"].as_u64().unwrap_or(0);
            }
            // An `error` event arrives mid-stream with a 200 already on
            // the wire, so it cannot surface as an HTTP status.
            Some("error") => {
                error = Some(format!(
                    "anthropic stream: {}",
                    truncate(event["error"]["message"].as_str().unwrap_or("unknown"), 200)
                ));
                break;
            }
            _ => {}
        }
    }
    // Bill whatever the stream did report, even on a mid-stream failure:
    // those tokens were generated and will be invoiced.
    bill(
        model,
        input_tokens,
        output_tokens,
        cache_creation,
        cache_read,
    );
    if let Some(e) = error {
        return failed(e);
    }
    Ok(text)
}

// ── Provider: OpenAI and compatibles (stryke ai.rs:1109-1173) ──────────

fn call_openai(call: &Call, url: &str, key_env: &str) -> Result<String> {
    // Local OpenAI-compatible servers (LM Studio, llama-server, vLLM)
    // usually want no auth at all, so a missing key is not fatal here —
    // send the request bare and let the server object if it cares.
    let api_key = std::env::var(key_env).unwrap_or_default();

    let mut messages = Vec::new();
    if !call.system.is_empty() {
        messages.push(serde_json::json!({"role": "system", "content": call.system}));
    }
    messages.push(serde_json::json!({"role": "user", "content": call.prompt}));

    let mut body = serde_json::json!({
        "model": call.model,
        "max_tokens": call.max_tokens,
        "messages": messages,
    });
    if call.temperature >= 0.0 {
        body["temperature"] = serde_json::Value::from(call.temperature);
    }

    let mut req = agent(call.timeout)
        .post(url)
        .set("content-type", "application/json");
    if !api_key.is_empty() {
        req = req.set("authorization", &format!("Bearer {}", api_key));
    }
    let json: serde_json::Value = req
        .send_json(body)
        .map_err(|e| AiError::Failed(format!("openai request: {}", e)))?
        .into_json()
        .map_err(|e| AiError::Failed(format!("openai decode: {}", e)))?;

    let usage = &json["usage"];
    bill(
        &call.model,
        usage["prompt_tokens"].as_u64().unwrap_or(0),
        usage["completion_tokens"].as_u64().unwrap_or(0),
        0,
        0,
    );

    let text = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return failed(format!(
            "openai returned no content: {}",
            truncate(&json.to_string(), 200)
        ));
    }
    Ok(text)
}

// ── Provider: Ollama (stryke ai.rs:3447-3505) ──────────────────────────

fn call_ollama(call: &Call) -> Result<String> {
    let base = {
        let configured = runtime().lock().base_url.clone();
        if !configured.is_empty() {
            configured
        } else {
            std::env::var("OLLAMA_HOST")
                .map(|h| {
                    if h.starts_with("http") {
                        h
                    } else {
                        format!("http://{}", h)
                    }
                })
                .unwrap_or_else(|_| "http://localhost:11434".to_string())
        }
    };
    let url = format!("{}/api/generate", base.trim_end_matches('/'));

    // Ollama needs an Ollama-tagged model name. If the model still
    // carries a hosted-provider default the user never overrode, swap in
    // a small local one rather than sending a name Ollama cannot resolve.
    let model = if call.model.is_empty()
        || call.model.starts_with("claude")
        || call.model.starts_with("gpt")
    {
        "llama3.2"
    } else {
        &call.model
    };

    let mut body = serde_json::json!({
        "model": model,
        "prompt": call.prompt,
        "stream": false,
        "options": { "num_predict": call.max_tokens },
    });
    if !call.system.is_empty() {
        body["system"] = serde_json::Value::String(call.system.clone());
    }
    if call.temperature >= 0.0 {
        body["options"]["temperature"] = serde_json::Value::from(call.temperature);
    }

    let json: serde_json::Value = agent(call.timeout)
        .post(&url)
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(|e| AiError::Failed(format!("ollama request: {}", e)))?
        .into_json()
        .map_err(|e| AiError::Failed(format!("ollama decode: {}", e)))?;

    // Tokens are tracked, cost is not: the model ran on this machine.
    INPUT_TOKENS.fetch_add(
        json["prompt_eval_count"].as_u64().unwrap_or(0),
        Ordering::Relaxed,
    );
    OUTPUT_TOKENS.fetch_add(json["eval_count"].as_u64().unwrap_or(0), Ordering::Relaxed);

    let text = json["response"].as_str().unwrap_or("").to_string();
    if text.is_empty() {
        return failed(format!(
            "ollama returned no content: {}",
            truncate(&json.to_string(), 200)
        ));
    }
    Ok(text)
}

// ── Provider: Gemini (stryke ai.rs:3511-3585) ──────────────────────────

fn call_gemini(call: &Call) -> Result<String> {
    let api_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_KEY"))
        .map_err(|_| AiError::Failed("$GOOGLE_API_KEY (or $GEMINI_API_KEY) is not set".into()))?;

    let model = if call.model.starts_with("claude") || call.model.starts_with("gpt") {
        "gemini-2.5-flash"
    } else {
        &call.model
    };
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let mut generation_config = serde_json::json!({ "maxOutputTokens": call.max_tokens });
    if call.temperature >= 0.0 {
        generation_config["temperature"] = serde_json::Value::from(call.temperature);
    }
    let mut body = serde_json::json!({
        "contents": [{ "parts": [{ "text": call.prompt }] }],
        "generationConfig": generation_config,
    });
    if !call.system.is_empty() {
        body["systemInstruction"] = serde_json::json!({ "parts": [{ "text": call.system }] });
    }

    let json: serde_json::Value = agent(call.timeout)
        .post(&url)
        .set("content-type", "application/json")
        .send_json(body)
        .map_err(|e| AiError::Failed(format!("gemini request: {}", e)))?
        .into_json()
        .map_err(|e| AiError::Failed(format!("gemini decode: {}", e)))?;

    let usage = &json["usageMetadata"];
    bill(
        model,
        usage["promptTokenCount"].as_u64().unwrap_or(0),
        usage["candidatesTokenCount"].as_u64().unwrap_or(0),
        0,
        0,
    );

    let mut text = String::new();
    if let Some(parts) = json["candidates"][0]["content"]["parts"].as_array() {
        for part in parts {
            if let Some(t) = part["text"].as_str() {
                text.push_str(t);
            }
        }
    }
    if text.is_empty() {
        return failed(format!(
            "gemini returned no content: {}",
            truncate(&json.to_string(), 200)
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every counter and table in this module is process-global, so the
    /// tests have to run one at a time against a known-zero baseline.
    fn reset() {
        COST_USD_MICROS.store(0, Ordering::Relaxed);
        INPUT_TOKENS.store(0, Ordering::Relaxed);
        OUTPUT_TOKENS.store(0, Ordering::Relaxed);
        CACHE_CREATION_TOKENS.store(0, Ordering::Relaxed);
        CACHE_READ_TOKENS.store(0, Ordering::Relaxed);
        CACHE_HITS.store(0, Ordering::Relaxed);
        CACHE_MISSES.store(0, Ordering::Relaxed);
        cache().lock().clear();
        mocks().lock().clear();
        history().lock().clear();
    }

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn opus_5_is_priced_at_the_published_rate_not_the_stryke_claude_3_rate() {
        // The regression this pins: stryke's table (ai.rs:143) prices
        // any `claude-opus*` at $15/$75 per MTok, which over-bills Opus 5
        // by 3x and would trip `max_cost_run` two-thirds early.
        assert_eq!(price_per_mtok("claude-opus-5"), (5.00, 25.00));
        assert_eq!(price_per_mtok("claude-sonnet-5"), (2.00, 10.00));
        assert_eq!(price_per_mtok("claude-haiku-4-5"), (1.00, 5.00));
        assert_eq!(price_per_mtok("claude-fable-5-1"), (10.00, 50.00));
        // Sonnet 4.6 keeps its own rate and must not fall into the
        // generic `claude-sonnet` arm below it.
        assert_eq!(price_per_mtok("claude-sonnet-4-6"), (3.00, 15.00));
        // An unknown model bills at Sonnet-class rates, which over-states
        // a cheap model rather than under-stating an expensive one.
        assert_eq!(price_per_mtok("some-model-from-2031"), (3.00, 15.00));
    }

    #[test]
    fn cache_tokens_bill_at_write_and_read_multipliers() {
        reset();
        // 1M input, 0 output, 1M cache-write, 1M cache-read on Opus 5:
        // 5.00 + 0 + 5.00*1.25 + 5.00*0.10 = 11.75.
        bill("claude-opus-5", 1_000_000, 0, 1_000_000, 1_000_000);
        assert!(
            (current_cost_usd() - 11.75).abs() < 0.000_01,
            "got {}",
            current_cost_usd()
        );
        assert_eq!(CACHE_CREATION_TOKENS.load(Ordering::Relaxed), 1_000_000);
        assert_eq!(CACHE_READ_TOKENS.load(Ordering::Relaxed), 1_000_000);
    }

    #[test]
    fn cache_key_is_unambiguous_across_field_boundaries() {
        // Without the NUL separators these two would hash identically,
        // and a system-prompt change would silently reuse the old answer.
        assert_ne!(
            cache_key("anthropic", "m", "ab", "c"),
            cache_key("anthropic", "m", "a", "bc")
        );
        assert_eq!(
            cache_key("anthropic", "m", "s", "p"),
            cache_key("anthropic", "m", "s", "p")
        );
    }

    #[test]
    fn flags_parse_into_the_right_slots() {
        let f = parse_flags(&args(&[
            "-m",
            "claude-sonnet-5",
            "-s",
            "be terse",
            "-t",
            "128",
            "-T",
            "0.2",
            "-n",
            "-b",
            "-v",
            "reply",
            "hello",
            "world",
        ]))
        .expect("parse");
        assert_eq!(f.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(f.system.as_deref(), Some("be terse"));
        assert_eq!(f.max_tokens, Some(128));
        assert_eq!(f.temperature, Some(0.2));
        assert!(f.no_cache && f.buffered);
        assert_eq!(f.var.as_deref(), Some("reply"));
        assert_eq!(f.words, vec!["hello", "world"]);
    }

    #[test]
    fn double_dash_stops_flag_parsing_so_a_prompt_may_start_with_a_dash() {
        let f = parse_flags(&args(&["--", "-m", "is not a flag here"])).expect("parse");
        assert_eq!(f.words, vec!["-m", "is not a flag here"]);
        assert!(f.model.is_none());
    }

    #[test]
    fn an_unknown_flag_is_a_usage_error_not_a_prompt_word() {
        // Silently prompting with "-Z" would burn a paid call on a typo.
        let err = parse_flags(&args(&["-Z", "hello"])).expect_err("should reject");
        assert_eq!(err.status(), 2);
        assert!(err.reason().contains("-Z"), "{}", err.reason());
    }

    #[test]
    fn a_flag_missing_its_argument_is_a_usage_error() {
        let err = parse_flags(&args(&["hello", "-m"])).expect_err("should reject");
        assert_eq!(err.status(), 2);
        assert!(err.reason().contains("-m"), "{}", err.reason());
    }

    #[test]
    fn mock_mode_answers_without_touching_the_network() {
        reset();
        std::env::set_var("ZSHRS_AI_MODE", "mock-only");
        mocks().lock().push((
            regex::Regex::new("capital of France").unwrap(),
            "Paris".into(),
        ));

        let out = run(&args(&["-v", "reply", "what is the capital of France?"]))
            .expect("mock should answer");
        match out {
            Output::Scalar(var, text) => {
                assert_eq!(var, "reply");
                assert_eq!(text, "Paris");
            }
            _ => panic!("expected a scalar assignment"),
        }
        // A mock is free and is not a cache hit.
        assert_eq!(current_cost_usd(), 0.0);
        assert_eq!(history().lock().len(), 1);
        std::env::remove_var("ZSHRS_AI_MODE");
    }

    #[test]
    fn mock_only_mode_refuses_rather_than_calling_out() {
        reset();
        std::env::set_var("ZSHRS_AI_MODE", "mock-only");
        let err = run(&args(&["-v", "x", "nothing matches this"])).expect_err("should refuse");
        assert_eq!(err.status(), 1);
        assert!(err.reason().contains("mock-only"), "{}", err.reason());
        std::env::remove_var("ZSHRS_AI_MODE");
    }

    #[test]
    fn array_output_splits_the_response_on_lines() {
        reset();
        std::env::set_var("ZSHRS_AI_MODE", "mock-only");
        mocks().lock().push((
            regex::Regex::new("^list").unwrap(),
            "one\ntwo\nthree".into(),
        ));

        match run(&args(&["-a", "items", "list three things"])).expect("mock") {
            Output::Array(var, lines) => {
                assert_eq!(var, "items");
                assert_eq!(lines, vec!["one", "two", "three"]);
            }
            _ => panic!("expected an array assignment"),
        }
        std::env::remove_var("ZSHRS_AI_MODE");
    }

    #[test]
    fn v_and_a_together_are_rejected() {
        let err = run(&args(&["-v", "s", "-a", "arr", "hi"])).expect_err("should reject");
        assert_eq!(err.status(), 2);
    }

    #[test]
    fn sse_decode_assembles_deltas_and_bills_the_whole_stream() {
        reset();
        // The billing bug this pins: stryke keeps the token counters as
        // locals inside `next_item`, so only the final chunk's usage is
        // ever charged. Here `message_start` and `message_delta` sit at
        // opposite ends of the body and both must land.
        let body = concat!(
            "event: message_start\n",
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":1000000,"cache_read_input_tokens":0}}}"#,
            "\n\n",
            "event: content_block_delta\n",
            r#"data: {"type":"content_block_delta","delta":{"text":"Hello, "}}"#,
            "\n\n",
            r#"data: {"type":"content_block_delta","delta":{"text":"world"}}"#,
            "\n\n",
            r#"data: {"type":"message_delta","usage":{"output_tokens":1000000}}"#,
            "\n\n",
        );
        let mut sink = Sink::Buffer(String::new());
        let text = decode_sse("claude-opus-5", std::io::Cursor::new(body), &mut sink)
            .expect("stream should decode");

        assert_eq!(text, "Hello, world");
        match sink {
            Sink::Buffer(b) => assert_eq!(b, "Hello, world"),
            _ => panic!("expected the buffer sink"),
        }
        assert_eq!(INPUT_TOKENS.load(Ordering::Relaxed), 1_000_000);
        assert_eq!(OUTPUT_TOKENS.load(Ordering::Relaxed), 1_000_000);
        // 1M in at $5 + 1M out at $25.
        assert!(
            (current_cost_usd() - 30.0).abs() < 0.000_01,
            "got {}",
            current_cost_usd()
        );
    }

    #[test]
    fn a_mid_stream_error_event_fails_the_call_but_still_bills_what_arrived() {
        reset();
        let body = concat!(
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":1000000}}}"#,
            "\n\n",
            r#"data: {"type":"content_block_delta","delta":{"text":"partial"}}"#,
            "\n\n",
            r#"data: {"type":"error","error":{"message":"overloaded_error"}}"#,
            "\n\n",
        );
        let mut sink = Sink::Buffer(String::new());
        let err = decode_sse("claude-opus-5", std::io::Cursor::new(body), &mut sink)
            .expect_err("an error event must fail the call");
        assert_eq!(err.status(), 1);
        assert!(
            err.reason().contains("overloaded_error"),
            "{}",
            err.reason()
        );
        // Those input tokens were consumed and will be invoiced.
        assert_eq!(INPUT_TOKENS.load(Ordering::Relaxed), 1_000_000);
    }

    #[test]
    fn sse_ignores_comment_and_event_lines_and_undecodable_payloads() {
        reset();
        let body = concat!(
            ": ping\n",
            "event: content_block_delta\n",
            "data: {not json at all\n",
            r#"data: {"type":"content_block_delta","delta":{"text":"ok"}}"#,
            "\n\n",
        );
        let mut sink = Sink::Buffer(String::new());
        let text = decode_sse("claude-opus-5", std::io::Cursor::new(body), &mut sink)
            .expect("garbage lines are skipped, not fatal");
        assert_eq!(text, "ok");
    }

    #[test]
    fn anthropic_body_sets_cache_control_only_with_dash_k() {
        let mut call = Call {
            provider: "anthropic".into(),
            model: "claude-opus-5".into(),
            system: "be terse".into(),
            prompt: "hi".into(),
            max_tokens: 64,
            temperature: -1.0,
            timeout: 30,
            cache: true,
            cache_control: false,
            stream: false,
        };
        let plain = anthropic_body(&call, false);
        assert!(plain["system"].is_string());
        assert!(
            plain.get("temperature").is_none(),
            "unset temp must be absent"
        );
        assert!(plain.get("stream").is_none());

        call.cache_control = true;
        call.temperature = 0.0;
        let cached = anthropic_body(&call, true);
        assert_eq!(
            cached["system"][0]["cache_control"]["type"].as_str(),
            Some("ephemeral")
        );
        // An explicit 0.0 must be sent; only a negative means "omit".
        assert_eq!(cached["temperature"].as_f64(), Some(0.0));
        assert_eq!(cached["stream"].as_bool(), Some(true));
    }

    #[test]
    fn retry_policy_matches_the_stryke_port() {
        for code in [429, 500, 502, 503, 504] {
            assert!(should_retry(code), "{} should retry", code);
        }
        for code in [400, 401, 403, 404, 413, 422] {
            assert!(!should_retry(code), "{} must not retry", code);
        }
        assert_eq!(retry_delay(0), Duration::from_secs(1));
        assert_eq!(retry_delay(3), Duration::from_secs(8));
        assert_eq!(retry_delay(9), Duration::from_secs(30), "capped at 30s");
    }

    #[test]
    fn config_set_rejects_an_unknown_key_instead_of_dropping_it() {
        let err = config_set("modle=claude-opus-5").expect_err("typo must not pass");
        assert_eq!(err.status(), 2);
        assert!(err.reason().contains("modle"), "{}", err.reason());
        assert!(config_set("noequals").is_err());
    }
}

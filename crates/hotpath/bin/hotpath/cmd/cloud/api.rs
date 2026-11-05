//! Shared plumbing of the `hotpath cloud` commands: token, HTTP client and
//! JSON output. The token comes only from `HOTPATH_API_TOKEN` (never a flag,
//! so it stays out of shell history and `ps`) and is never printed. A 2xx body
//! a command only prints passes through as received (`RawBody`), so unknown
//! fields still reach the caller. Every failure is one JSON document on stderr
//! (`CliError`). Exit codes: 0 ok, 1 error, 2 clap usage error.

use std::io::Write;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use hotpath::json::cloud_api::API_URL;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::Serialize;
use serde_json::{json, Value};

pub(crate) const TOKENS_URL: &str = "https://hotpath.rs/app/tokens";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Longest raw (unparseable) response body quoted in a message.
const MAX_QUOTED_BODY: usize = 200;
/// Every byte of a query value but RFC 3986's unreserved ones is
/// percent-encoded: a `+` of a time offset must not reach the server as a space.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `HOTPATH_API_TOKEN`, trimmed; `None` when unset or blank.
static API_TOKEN: LazyLock<Option<String>> = LazyLock::new(|| {
    std::env::var("HOTPATH_API_TOKEN")
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
});

/// One failure of a `hotpath cloud` command, rendered by `Output::emit_error`.
#[derive(Debug)]
pub(crate) enum CliError {
    /// A non-2xx server body that is valid JSON, kept verbatim so compact
    /// output prints exactly what the server sent.
    Server(String),
    /// A failure with no server body to show; prints as `{"error": message}`.
    Client(String),
}

impl CliError {
    pub(crate) fn client(message: impl Into<String>) -> Self {
        Self::Client(message.into())
    }

    /// A non-2xx body: passed through when it is JSON, otherwise quoted
    /// with the status.
    fn server(status: u16, body: String) -> Self {
        match serde_json::from_str::<Value>(&body) {
            Ok(_) => Self::Server(body.trim().to_string()),
            Err(_) => Self::client(format!("HTTP {status}: {}", quote_body(&body))),
        }
    }
}

/// A 2xx body that is valid JSON, kept exactly as received.
pub(crate) struct RawBody {
    url: String,
    body: String,
}

impl RawBody {
    /// The body parsed as `T`, the part of it a command branches on; failing
    /// like `Client::get` does.
    pub(crate) fn parse<T: DeserializeOwned>(&self) -> Result<T, CliError> {
        parse_body(&self.url, &self.body)
    }
}

pub(crate) struct Client {
    base_url: String,
    /// `None` for an anonymous client: no `Authorization` header is sent.
    token: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    /// A client that authenticates every request with `HOTPATH_API_TOKEN`.
    pub(crate) fn from_env() -> Result<Self, CliError> {
        let token = API_TOKEN.clone().ok_or_else(|| {
            CliError::client(format!(
                "HOTPATH_API_TOKEN is not set. Create a token at {TOKENS_URL} and export it."
            ))
        })?;
        Ok(Self::new(Some(token)))
    }

    /// A client for public routes: it never reads the token, so none is
    /// required and none is sent.
    pub(crate) fn anonymous() -> Self {
        Self::new(None)
    }

    fn new(token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .user_agent(concat!("hotpath-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            base_url: API_URL.clone(),
            token,
            agent,
        }
    }

    /// `GET {base_url}{path}` with the bearer token, if any; a 2xx body parses as `T`,
    /// anything else is the server body as the error (see `CliError::server`).
    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, CliError> {
        let url = format!("{}{path}", self.base_url);
        let body = self.get_body(&url)?;
        parse_body(&url, &body)
    }

    /// `GET {base_url}{path}` like `get`, with the 2xx body kept as received:
    /// for a command that only prints it.
    pub(crate) fn get_raw(&self, path: &str) -> Result<RawBody, CliError> {
        let url = format!("{}{path}", self.base_url);
        let body = self.get_body(&url)?;
        parse_body::<IgnoredAny>(&url, &body)?;
        Ok(RawBody { url, body })
    }

    fn get_body(&self, url: &str) -> Result<String, CliError> {
        let resp = self.authorize(self.agent.get(url)).call();
        self.read_response(url, resp)
    }

    /// `POST {base_url}{path}` with `body` as JSON and the bearer token, if
    /// any; the answer is handled like `get`'s.
    pub(crate) fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, CliError> {
        let url = format!("{}{path}", self.base_url);
        let body = serde_json::to_vec(body)
            .map_err(|e| CliError::client(format!("could not serialize the request: {e}")))?;
        let resp = self
            .authorize(self.agent.post(&url))
            .header("Content-Type", "application/json")
            .send(&body[..]);
        let body = self.read_response(&url, resp)?;
        parse_body(&url, &body)
    }

    /// `request` with the bearer token; unchanged for an anonymous client.
    fn authorize<B>(&self, request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        match &self.token {
            Some(token) => request.header("Authorization", &format!("Bearer {token}")),
            None => request,
        }
    }

    /// A 2xx body as received, or the failure: the transport error (which
    /// names the base URL only) or the non-2xx body (see `CliError::server`).
    fn read_response(
        &self,
        url: &str,
        resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<String, CliError> {
        let mut resp = resp
            .map_err(|e| CliError::client(format!("request to {} failed: {e}", self.base_url)))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string();
        if (200..300).contains(&status) {
            return body.map_err(|e| CliError::client(format!("invalid response from {url}: {e}")));
        }
        Err(CliError::server(status, body.unwrap_or_default()))
    }
}

/// `path` with `params` as its query string, in order, each value
/// percent-encoded; `path` alone without any. Keys are the caller's literals.
pub(crate) fn with_query(path: &str, params: &[(&str, &str)]) -> String {
    let mut url = path.to_string();
    for (i, (key, value)) in params.iter().enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        url.push_str(key);
        url.push('=');
        url.extend(utf8_percent_encode(value, QUERY_VALUE));
    }
    url
}

/// A 2xx body of `url` parsed as `T`; a body that does not parse is an error,
/// never a default.
fn parse_body<T: DeserializeOwned>(url: &str, body: &str) -> Result<T, CliError> {
    serde_json::from_str(body)
        .map_err(|e| CliError::client(format!("invalid response from {url}: {e}")))
}

fn quote_body(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return "empty response body".to_string();
    }
    let end = hotpath::floor_char_boundary(body, MAX_QUOTED_BODY);
    if end < body.len() {
        format!("{}...", &body[..end])
    } else {
        body.to_string()
    }
}

/// Where and how a command's JSON goes: compact on stdout unless `--pretty`
/// or `--output FILE` say otherwise. Errors always go to stderr, indented
/// under `--pretty` like a body.
pub(crate) struct Output {
    pub(crate) pretty: bool,
    pub(crate) file: Option<PathBuf>,
}

impl Output {
    pub(crate) fn emit<T: Serialize>(&self, value: &T) -> Result<(), CliError> {
        let json = self
            .render(value)
            .map_err(|e| CliError::client(format!("could not serialize the response: {e}")))?;
        self.write(json)
    }

    /// Writes a body as the server sent it (see `render_raw`).
    pub(crate) fn emit_raw(&self, body: &RawBody) -> Result<(), CliError> {
        self.write(self.render_raw(&body.body))
    }

    pub(crate) fn emit_error(&self, error: &CliError) {
        let json = match error {
            CliError::Server(raw) => self.render_raw(raw),
            CliError::Client(message) => {
                let value = json!({ "error": message });
                self.render(&value).unwrap_or_else(|_| value.to_string())
            }
        };
        eprintln!("{json}");
    }

    fn write(&self, mut json: String) -> Result<(), CliError> {
        json.push('\n');
        match &self.file {
            Some(path) => std::fs::write(path, json)
                .map_err(|e| CliError::client(format!("could not write {}: {e}", path.display()))),
            None => std::io::stdout()
                .write_all(json.as_bytes())
                .map_err(|e| CliError::client(format!("could not write to stdout: {e}"))),
        }
    }

    /// A server body: compact output is the body exactly as received
    /// (trimmed), `--pretty` re-indents it in the server's key order
    /// (`serde_json/preserve_order` under the `cloud` feature).
    fn render_raw(&self, raw: &str) -> String {
        let raw = raw.trim();
        if !self.pretty {
            return raw.to_string();
        }
        serde_json::from_str::<Value>(raw)
            .and_then(|value| self.render(&value))
            .unwrap_or_else(|_| raw.to_string())
    }

    fn render<T: Serialize>(&self, value: &T) -> serde_json::Result<String> {
        if self.pretty {
            serde_json::to_string_pretty(value)
        } else {
            serde_json::to_string(value)
        }
    }
}

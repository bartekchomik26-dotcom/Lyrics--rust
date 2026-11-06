//! HTTPS through `ureq`, as Horizon's own account code uses it.

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

pub const USER_AGENT: &str = concat!(
    "LyricsPlus-Horizon/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/horizon-rust/horizon-mods)"
);

/// What a request came back with.
#[derive(Debug)]
pub enum HttpError {
    /// A status the server answered with, its `Retry-After` seconds and body.
    Status(u16, Option<u64>, String),
    /// No answer: DNS, TLS, timeout.
    Transport(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Status(code, _, body) => {
                let body: String = body.chars().take(160).collect();
                write!(f, "HTTP {code}: {body}")
            }
            HttpError::Transport(e) => write!(f, "{e}"),
        }
    }
}

pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(6))
            .timeout(Duration::from_secs(12))
            .user_agent(USER_AGENT)
            .build()
    })
}

fn map(result: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, HttpError> {
    match result {
        Ok(r) => Ok(r),
        Err(ureq::Error::Status(code, r)) => {
            let retry = r.header("Retry-After").and_then(|v| v.trim().parse().ok());
            Err(HttpError::Status(code, retry, r.into_string().unwrap_or_default()))
        }
        Err(e) => Err(HttpError::Transport(e.to_string())),
    }
}

/// A GET's status and body text.
pub fn get_text(url: &str, headers: &[(&str, &str)]) -> Result<(u16, String), HttpError> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let r = map(req.call())?;
    let status = r.status();
    let body = r
        .into_string()
        .map_err(|e| HttpError::Transport(e.to_string()))?;
    Ok((status, body))
}

/// A GET's JSON (`Value::Null` for an empty body such as 204).
pub fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<(u16, serde_json::Value), HttpError> {
    let (status, body) = get_text(url, headers)?;
    if body.trim().is_empty() {
        return Ok((status, serde_json::Value::Null));
    }
    let v = serde_json::from_str(&body).map_err(|e| HttpError::Transport(e.to_string()))?;
    Ok((status, v))
}

/// Up to `limit` bytes of a GET (album art).
pub fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, HttpError> {
    let r = map(agent().get(url).call())?;
    let mut out = Vec::new();
    r.into_reader()
        .take(limit)
        .read_to_end(&mut out)
        .map_err(|e| HttpError::Transport(e.to_string()))?;
    Ok(out)
}

/// A form POST's JSON answer.
pub fn post_form(url: &str, form: &[(&str, &str)]) -> Result<serde_json::Value, HttpError> {
    let r = map(agent().post(url).send_form(form))?;
    let body = r
        .into_string()
        .map_err(|e| HttpError::Transport(e.to_string()))?;
    serde_json::from_str(&body).map_err(|e| HttpError::Transport(e.to_string()))
}

/// An empty-bodied request with a bearer token (Spotify's player controls).
pub fn send_empty(method: &str, url: &str, token: &str) -> Result<u16, HttpError> {
    let auth = format!("Bearer {token}");
    let r = map(agent().request(method, url).set("Authorization", &auth).send_bytes(&[]))?;
    Ok(r.status())
}

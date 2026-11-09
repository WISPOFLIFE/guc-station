//! HTTP client for GUC's IIS hosts: cookies, manual redirects, and the NTLM
//! handshake. NTLM authenticates a TCP connection rather than a request, so
//! all traffic to one host is serialised to keep the handshake on a single
//! keep-alive connection.

use crate::ntlm;
use reqwest::{Method, StatusCode, header};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use url::Url;

const MAX_REDIRECTS: usize = 10;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) GUCStation/0.1";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Wrong username or password.")]
    BadCredentials,
    #[error("You're not signed in.")]
    NotSignedIn,
    #[error("Only https://*.guc.edu.eg addresses are allowed, got {0}")]
    ForbiddenHost(String),
    #[error("The server answered with HTTP {0}.")]
    Status(u16),
    #[error("Couldn't reach the server: {0}")]
    Network(#[from] reqwest::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Credentials {
    pub domain: String,
    pub username: String,
    pub password: String,
}

impl Credentials {
    /// Accepts `name`, `name@student.guc.edu.eg` or `DOMAIN\name`.
    pub fn parse(login: &str, password: &str) -> Self {
        let login = login.trim();
        let (domain, user) = match login.split_once('\\') {
            Some((d, u)) => (d.to_string(), u),
            None => (String::new(), login),
        };
        let username = user.split('@').next().unwrap_or(user).to_string();
        Credentials { domain, username, password: password.to_string() }
    }
}

pub enum Body {
    Empty,
    Form(Vec<(String, String)>),
    Xml(String),
}

pub struct Response {
    pub url: Url,
    pub status: StatusCode,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

pub struct GucClient {
    http: reqwest::Client,
    creds: Credentials,
    host_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Domain the server named in its last NTLM challenge, for diagnostics.
    ntlm_domain: Mutex<Option<String>>,
}

pub fn check_host(url: &Url) -> Result<()> {
    let ok = url.scheme() == "https"
        && url.host_str().is_some_and(|h| h == "guc.edu.eg" || h.ends_with(".guc.edu.eg"));
    if ok { Ok(()) } else { Err(Error::ForbiddenHost(url.to_string())) }
}

impl GucClient {
    pub fn new(creds: Credentials) -> Result<Self> {
        let http = reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .pool_max_idle_per_host(1)
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(GucClient { http, creds, host_locks: Mutex::new(HashMap::new()), ntlm_domain: Mutex::new(None) })
    }

    pub fn username(&self) -> &str {
        &self.creds.username
    }

    pub fn ntlm_domain(&self) -> Option<String> {
        self.ntlm_domain.lock().unwrap().clone()
    }

    pub async fn get(&self, url: &str) -> Result<Response> {
        let url = Url::parse(url).map_err(|e| Error::Other(e.to_string()))?;
        self.send(Method::GET, url, Body::Empty, &[]).await
    }

    pub async fn send(&self, method: Method, url: Url, body: Body, headers: &[(&str, &str)]) -> Result<Response> {
        let (mut method, mut url, mut body) = (method, url, body);
        for _ in 0..=MAX_REDIRECTS {
            check_host(&url)?;
            let resp = self.send_authenticated(&method, &url, &body, headers).await?;
            if !resp.status.is_redirection() {
                return match resp.status {
                    StatusCode::UNAUTHORIZED => Err(Error::BadCredentials),
                    s if s.is_server_error() => Err(Error::Status(s.as_u16())),
                    _ => Ok(resp),
                };
            }
            let location = String::from_utf8_lossy(&resp.bytes).into_owned();
            url = url.join(&location).map_err(|e| Error::Other(e.to_string()))?;
            if resp.status != StatusCode::TEMPORARY_REDIRECT && resp.status != StatusCode::PERMANENT_REDIRECT {
                method = Method::GET;
                body = Body::Empty;
            }
        }
        Err(Error::Other("Too many redirects.".into()))
    }

    /// One request/response, doing the NTLM dance if the server asks for it.
    /// Redirect responses come back with the `Location` header in `bytes`.
    async fn send_authenticated(&self, method: &Method, url: &Url, body: &Body, headers: &[(&str, &str)]) -> Result<Response> {
        let lock = self.host_lock(url.host_str().unwrap_or_default());
        let _guard = lock.lock().await;

        let first = self.raw(method, url, body, headers, None).await?;
        if first.status() != StatusCode::UNAUTHORIZED || !offers_ntlm(&first) {
            return finish(first).await;
        }
        drain(first).await;

        let challenge_resp = self
            .raw(method, url, body, headers, Some(format!("NTLM {}", ntlm::negotiate_message())))
            .await?;
        let challenge = challenge_resp
            .headers()
            .get_all(header::WWW_AUTHENTICATE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find_map(ntlm::parse_challenge);
        drain(challenge_resp).await;
        let challenge = challenge.ok_or_else(|| Error::Other("Server didn't send an NTLM challenge.".into()))?;

        *self.ntlm_domain.lock().unwrap() = Some(challenge.target_name().to_string());
        let c = &self.creds;
        let auth = ntlm::authenticate_message(&challenge, &c.domain, &c.username, &c.password);
        let resp = self.raw(method, url, body, headers, Some(format!("NTLM {auth}"))).await?;
        finish(resp).await
    }

    async fn raw(&self, method: &Method, url: &Url, body: &Body, headers: &[(&str, &str)], auth: Option<String>) -> Result<reqwest::Response> {
        let mut req = self.http.request(method.clone(), url.clone());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        if let Some(a) = auth {
            req = req.header(header::AUTHORIZATION, a);
        }
        req = match body {
            Body::Empty => req,
            Body::Form(fields) => req.form(fields),
            Body::Xml(xml) => {
                // Allows a slow upload of about 20 KB/s, so large attachments don't hit the usual minute.
                let seconds = (xml.len() as u64 / 20_000).max(60);
                req.header(header::CONTENT_TYPE, "text/xml; charset=utf-8").timeout(std::time::Duration::from_secs(seconds)).body(xml.clone())
            }
        };
        Ok(req.send().await?)
    }

    fn host_lock(&self, host: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.host_locks.lock().unwrap();
        locks.entry(host.to_string()).or_default().clone()
    }
}

fn offers_ntlm(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .any(|v| v.to_str().is_ok_and(|s| s.trim().eq_ignore_ascii_case("ntlm") || s.starts_with("NTLM")))
}

/// Reads the body to the end so the connection goes back to the pool.
async fn drain(resp: reqwest::Response) {
    let _ = resp.bytes().await;
}

async fn finish(resp: reqwest::Response) -> Result<Response> {
    let status = resp.status();
    let url = resp.url().clone();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let location = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).map(str::to_string);
    let bytes = resp.bytes().await?.to_vec();
    let bytes = match (status.is_redirection(), location) {
        (true, Some(loc)) => loc.into_bytes(),
        _ => bytes,
    };
    Ok(Response { url, status, content_type, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_login_forms() {
        let c = Credentials::parse("ahmed.ali@student.guc.edu.eg", "x");
        assert_eq!((c.domain.as_str(), c.username.as_str()), ("", "ahmed.ali"));
        let c = Credentials::parse("GUC\\ahmed.ali", "x");
        assert_eq!((c.domain.as_str(), c.username.as_str()), ("GUC", "ahmed.ali"));
    }

    #[test]
    fn only_guc_hosts() {
        assert!(check_host(&Url::parse("https://apps.guc.edu.eg/x").unwrap()).is_ok());
        assert!(check_host(&Url::parse("https://evil.com/guc.edu.eg").unwrap()).is_err());
        assert!(check_host(&Url::parse("http://apps.guc.edu.eg/").unwrap()).is_err());
    }
}

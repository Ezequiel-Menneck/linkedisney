use std::{
    io::{Read, Write},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use crate::models::Config;

const AUTH_URL: &str = "https://www.linkedin.com/oauth/v2/authorization";
const TOKEN_URL: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const USERINFO_URL: &str = "https://api.linkedin.com/v2/userinfo";
pub const SCOPES: &str = "openid profile email w_member_social";

pub struct LoggedIn {
    pub access_token: String,
    pub author_urn: String,
    pub name: String,
    pub expires_in: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Deserialize)]
struct UserInfo {
    sub: String,
    #[serde(default)]
    name: String,
}

/// state aleatório razoável (nanos + pid hash). Para produção, use um CSPRNG.
fn make_state() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let pid = std::process::id();
    format!("{:016x}{:04x}", nanos, pid ^ (nanos as u32))
}

/// Grip do redirect: env override útil p/ homologar sem rebuild
fn redirect_uri() -> String {
    std::env::var("LINKEDIN_REDIRECT_URI").unwrap_or_else(|_| crate::models::OAUTH_REDIRECT_URI.into())
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = Command::new("xdg-open").arg(url).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = url; // windows: use `cmd /c start`
}

fn url_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte as char);
                i += 3;
                continue;
            }
        }
        out.push(s[i..].chars().next().unwrap());
        i += 1;
    }
    out
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), url_decode(v)))
        })
        .collect()
}

pub async fn login(cfg: &Config) -> Result<LoggedIn> {
    if !cfg.oauth_ready() {
        bail!("LinkedIn_client_id/secret ausentes — configure em Configure (aba LinkedIn) ou no .env");
    }
    let client_id = cfg.li_client_id.trim();
    let client_secret = cfg.li_client_secret.trim();
    let port = crate::models::OAUTH_REDIRECT_PORT;
    let redirect_uri = redirect_uri();
    let state = make_state();

    let scope_encoded = SCOPES.replace(' ', "%20");
    // Padrão: URL-encoded (exigido pela doc). LINKEDIN_REDIRECT_URI_RAW=1 envia o valor cru.
    let raw_mode = std::env::var("LINKEDIN_REDIRECT_URI_RAW").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
    let redirect_param = if raw_mode {
        redirect_uri.clone()
    } else {
        redirect_uri.replace(':', "%3A").replace('/', "%2F")
    };
    println!(
        "\n↻ conferindo: o Redirect URL registrado no app do LinkedIn deve ser EXATAMENTE:\n  {redirect_uri}\n"
    );
    let auth_url = format!(
        "{AUTH_URL}?response_type=code&client_id={client_id}&redirect_uri={redirect_param}\
         &state={state}&scope={scope_encoded}"
    );

    println!("Abrindo navegador para login no LinkedIn...");
    println!("(se não abrir, cole esta URL no navegador):\n{auth_url}");
    open_browser(&auth_url);

    let code = wait_callback_blocking(port, &state)?;

    let resp = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .context("falha ao trocar o código pelo token")?;
    if !resp.status().is_success() {
        bail!("LinkedIn OAuth erro: {}", resp.text().await?);
    }
    let token: TokenResponse = resp.json().await?;

    let ui_resp = reqwest::Client::new()
        .get(USERINFO_URL)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .context("falha ao buscar userinfo")?;
    if !ui_resp.status().is_success() {
        bail!("LinkedIn userinfo erro: {}", ui_resp.text().await?);
    }
    let ui: UserInfo = ui_resp.json().await?;

    Ok(LoggedIn {
        access_token: token.access_token,
        author_urn: format!("urn:li:person:{}", ui.sub),
        name: ui.name,
        expires_in: token.expires_in,
    })
}

fn wait_callback_blocking(port: u16, state: &str) -> Result<String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("não consegui usar a porta {port} (é otra instância rodando?)"))?;
    println!("aguardando callback em http://localhost:{port}/callback ...");
    for _ in 0..40 {
        let (mut conn, _) = listener.accept()?;
        let mut buf = [0u8; 8192];
        let n = conn.read(&mut buf)?;
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let line = req.lines().next().unwrap_or("").to_string();
        let is_favicon = line.contains("/favicon.ico");
        conn.write_all(if is_favicon {
            b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n" as &[u8]
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
              <html><body><h2>Token recebido!</h2>Pode fechar esta aba e voltar ao terminal.</body></html>\r\n" as &[u8]
        })?;
        drop(conn);
        let path = line.split(' ').nth(1).unwrap_or("").to_string();
        println!("[callback] {path}"); // debug: o que o navegador mandou
        let Some(q) = path.split_once('?').map(|(_, g)| g) else { continue };
        let params = parse_query(q);
        if let Some((_, err)) = params.iter().find(|(k, _)| k == "error") {
            let desc = params
                .iter()
                .find(|(k, _)| k == "error_description")
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            return Err(anyhow!(
                "LinkedIn recusou a autorização: {err}\n  {desc}\n\
                 Dica comum: certifique-se de que o app tem os products ativados:\n\
                 1. 'Sign In with LinkedIn using OpenID Connect'  (scopes: openid profile email)\n\
                 2. 'Share on LinkedIn'  (scope: w_member_social)\n\
                 Veja: developer.linkedin.com/apps -> seu app -> Products"
            ));
        }
        let code = params.iter().find(|(k, _)| k == "code").map(|(_, v)| v.clone());
        let got_state = params.iter().find(|(k, _)| k == "state").map(|(_, v)| v.clone());
        if let Some(code) = code {
            if got_state.as_deref() == Some(state) {
                return Ok(code);
            }
            return Err(anyhow!("state divergente (possível CSRF) — tente de novo"));
        }
        println!("callback sem code? tentando de novo...");
    }
    bail!("timeout: callback não chegou")
}

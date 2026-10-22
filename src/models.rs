use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub ai_api_key: String,
    pub ai_base_url: String,
    pub ai_model: String,
    pub li_client_id: String,
    pub li_client_secret: String,
    pub linkedin_access_token: String,
    pub linkedin_author_urn: String,
    /// Versão mensal da API REST do LinkedIn (ex: "202608"); window devida.
    #[serde(default = "li_api_version_default")]
    pub li_api_version: String,
}

fn li_api_version_default() -> String {
    LINKEDIN_API_VERSION_DEFAULT.to_string()
}

pub const OAUTH_REDIRECT_PORT: u16 = 6555;
pub const OAUTH_REDIRECT_URI: &str = "http://localhost:6555/callback";

/// Versão mensal da API REST do LinkedIn (YYYYMM). Cada versão vive ~12 meses;
/// com um ano de janela, bump a cada ~6 meses. Ajuste via env LINKEDIN_API_VERSION.
pub const LINKEDIN_API_VERSION_DEFAULT: &str = "202608";

impl Config {
    /// Resolve a conexão com a IA na ordem: config.json -> env vars -> defaults.
    pub fn resolved(&self) -> Config {
        let mut c = self.clone();
        if c.ai_api_key.is_empty() {
            if let Ok(k) = std::env::var("AI_API_KEY").or_else(|_| std::env::var("OPENAI_API_KEY")) {
                c.ai_api_key = k;
            }
        }
        if c.ai_base_url.is_empty() {
            c.ai_base_url =
                std::env::var("AI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        }
        if c.ai_model.is_empty() {
            c.ai_model = std::env::var("AI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        }
        if c.li_client_id.is_empty() {
            if let Ok(v) = std::env::var("LINKEDIN_CLIENT_ID") {
                c.li_client_id = v;
            }
        }
        if c.li_client_secret.is_empty() {
            if let Ok(v) = std::env::var("LINKEDIN_CLIENT_SECRET") {
                c.li_client_secret = v;
            }
        }
        if c.li_api_version.trim().is_empty() {
            c.li_api_version = std::env::var("LINKEDIN_API_VERSION")
                .unwrap_or_else(|_| LINKEDIN_API_VERSION_DEFAULT.to_string());
        }
        c
    }

    pub fn ai_ready(&self) -> bool {
        !self.ai_api_key.trim().is_empty() && !self.ai_base_url.trim().is_empty()
    }

    pub fn oauth_ready(&self) -> bool {
        !self.li_client_id.trim().is_empty() && !self.li_client_secret.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub industry: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub topics: String,
    #[serde(default)]
    pub tone: String,
    #[serde(default)]
    pub language: String,
    /// Voice: preferências de escrita (ritmo, vocabulário, o que evitar).
    #[serde(default)]
    pub voice_prefs: String,
    /// Strategy: objetivos e posicionamento desejado no LinkedIn.
    #[serde(default)]
    pub goals: String,
}

/// Registro da base de conhecimento pessoal (sempre fornecido pelo usuário).
#[derive(Debug, Clone)]
pub struct Fact {
    pub id: i64,
    /// "fact" (matéria-prima factual) ou "preference" (feedback/preferência editorial).
    pub kind: String,
    pub content: String,
    /// Temas associados, separados por vírgula.
    pub topics: String,
}

pub const FACT: &str = "fact";
pub const PREF: &str = "preference";

impl Fact {
    pub fn is_preference(&self) -> bool {
        self.kind == PREF
    }
}

#[derive(Debug, Clone)]
pub struct Post {
    pub id: i64,
    pub content: String,
    pub status: String,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub posted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

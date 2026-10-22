use anyhow::{Context, Result};
use serde_json::json;

use crate::models::Config;

/// Limite de caracteres aceitos pelo LinkedIn em post de texto.
pub const MAX_COMMENTARY_CHARS: usize = 3000;

/// Normalização conservadora do commentary antes de publicar:
/// - remove cercas de código Markdown acidentais (``` ou ```linguagem) sem
///   tocar no resto do conteúdo;
/// - reduz runs de 3 ou mais quebras de linha para uma linha em branco só
///   (parágrafo separado continua separado);
/// - trim de espaços/linhas em branco nas pontas;
/// - NÃO converte Markdown→HTML e NÃO remove caracteres legítimos
///   (*, #, -, [], crases no meio de texto, etc).
pub fn normalize_commentary(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            !(t == "```" || (t.starts_with("```") && t[3..].chars().all(|c| c.is_alphanumeric())))
        })
        .collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut blank_run = 0usize;
    for line in lines {
        if line.trim().is_empty() {
            blank_run += 1;
            // no máximo UMA linha em branco entre parágrafos
            if blank_run <= 1 {
                out.push(String::new());
            }
        } else {
            blank_run = 0;
            out.push(line.to_string());
        }
    }
    while out.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.remove(0);
    }
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    // trim das pontas (mantém espaços em branco internos intactos)
    out.join("\n")
        .trim()
        .trim_matches('\n')
        .to_string()
        .trim()
        .to_string()
}

/// Escapa os caracteres reservados do formato "little text" do LinkedIn
/// (Posts API). Sem escape, a API trunca silenciosamente o commentary no
/// primeiro caractere reservado (ex: "(" de um parêntese).
fn escape_little_text(s: &str) -> String {
    const RESERVED: [char; 15] = ['\\', '|', '{', '}', '@', '[', ']', '(', ')', '<', '>', '#', '*', '_', '~'];
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if RESERVED.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub async fn create_text_post(cfg: &Config, content: &str) -> Result<String> {
    // normaliza artefatos de Markdown e valida limites antes de ir pra API
    let content = normalize_commentary(content);
    if content.trim().is_empty() {
        anyhow::bail!("conteúdo do post vazio após normalização");
    }
    if content.chars().count() > MAX_COMMENTARY_CHARS {
        anyhow::bail!(
            "post excede o limite da API do LinkedIn ({} > {} caracteres)",
            content.chars().count(),
            MAX_COMMENTARY_CHARS
        );
    }
    let url = "https://api.linkedin.com/rest/posts";
    let body = json!({
        "author": cfg.linkedin_author_urn,
        "commentary": escape_little_text(&content),
        "visibility": "PUBLIC",
        "distribution": { "feedDistribution": "MAIN_FEED", "targetEntities": [] },
        "lifecycleState": "PUBLISHED",
        "isReshareDisabledByAuthor": false
    });

    let client = reqwest::Client::new();
    crate::log::debug("linkedin: POST /rest/posts");
    let resp = client
        .post(url)
        .timeout(std::time::Duration::from_secs(30))
        .header("Authorization", format!("Bearer {}", cfg.linkedin_access_token))
        .header("Content-Type", "application/json")
        .header("X-Restli-Protocol-Version", "2.0.0")
        .header("LinkedIn-Version", cfg.li_api_version.trim())
        .json(&body)
        .send()
        .await
        .context("request to LinkedIn API failed")?;

    let status = resp.status();
    let resp_h = resp.headers().clone();
    let text = resp.text().await?;

    if !status.is_success() {
        crate::log::error(&format!("linkedin: {status}: {text}"));
        anyhow::bail!("LinkedIn API error ({status}): {text}");
    }
    let post_id = resp_h
        .get("x-restli-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    crate::log::info(&format!("linkedin: post publicado (external id {post_id})"));
    Ok(post_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_reserved_characters_single_pass() {
        assert_eq!(escape_little_text("(18%)"), "\\(18%\\)");
        assert_eq!(escape_little_text("back \\ slash"), "back \\\\ slash");
        assert_eq!(escape_little_text("#rust @man"), "\\#rust \\@man");
        assert_eq!(escape_little_text("`code`"), "`code`"); // não reservado
        assert_eq!(escape_little_text("normal"), "normal");
    }

    #[test]
    fn normaliza_texto_puro_vira_ele_mesmo() {
        // texto já em plain text: intacto (hashtag, url, acento, emoji, bullet)
        let t = "Post sobre Java ☕\n• ponto um\n→ ponto dois\n#Java #SoftwareEngineering\nhttps://exemplo.com/doc";
        assert_eq!(normalize_commentary(t), t);
    }

    #[test]
    fn remove_cercas_de_codigo_acidentais() {
        let t = "```text\nlinha um\n```";
        assert_eq!(normalize_commentary(t), "linha um");
        let t = "\n```python\nprint(1)\n```\n";
        assert_eq!(normalize_commentary(t), "print(1)");
        // cerca com conteúdo além do nome de linguagem NÃO é removida (conservadorismo)
        let t = "```isso aqui é texto legítimo";
        assert_eq!(normalize_commentary(t), "```isso aqui é texto legítimo");
    }

    #[test]
    fn preserva_paragrafos_e_colapsa_excesso() {
        let t = "parágrafo 1\n\n\n\nparágrafo 2\n\nparágrafo 3";
        assert_eq!(normalize_commentary(t), "parágrafo 1\n\nparágrafo 2\n\nparágrafo 3");
    }

    #[test]
    fn preserva_unicode_emojis_acentos() {
        let t = "Coração ❤️  acentuação ção 汉字";
        assert_eq!(normalize_commentary(t), t);
        assert_eq!(normalize_commentary(t).chars().count(), t.chars().count());
    }

    #[test]
    fn trim_e_linhas_vazias_nas_pontas() {
        assert_eq!(normalize_commentary("\n\n   inicio\n\n\n"), "inicio");
        assert_eq!(normalize_commentary("   \n\n"), "");
    }

    #[test]
    fn limite_vazio_e_estouro() {
        assert!(normalize_commentary("").is_empty());
        assert!(normalize_commentary("\n``` \n```\n").is_empty());
        assert!(format!("{}", "a".repeat(MAX_COMMENTARY_CHARS + 1)).chars().count() > MAX_COMMENTARY_CHARS);
        assert!(format!("{}", "a".repeat(MAX_COMMENTARY_CHARS)).chars().count() <= MAX_COMMENTARY_CHARS);
    }

    #[test]
    fn escapamento_e_normalizacao_coexistem() {
        // pipeline real: normalize → escape; quebras de linha preservadas
        let t = "```text\nPonto (importante):\n#line\n```";
        let n = normalize_commentary(t);
        assert_eq!(n, "Ponto (importante):\n#line");
        let e = escape_little_text(&n);
        assert!(e.contains("\\(importante\\)"));
        assert!(e.contains("\\n") || e.contains('\n')); // \n literal preservado pelo serde json
    }

    #[test]
    fn limite_da_api_maior_que_o_editorial() {
        assert!(MAX_COMMENTARY_CHARS > 1800); // geração fica sob o teto de 1800
    }
}

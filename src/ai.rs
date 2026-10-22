use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::models::{Config, Fact, Post, Profile};

/// Limite de caracteres do post final (inclui espaços e quebras de linha).
pub const MAX_POST_CHARS: usize = 1800;
/// Teto de fatos enviados por request (seleção por relevância).
pub const MAX_FACTS: usize = 6;
/// Teto de preferências editoriais enviadas por request.
pub const MAX_PREFS: usize = 10;
/// Teto de exemplos de escrita enviados por request.
pub const MAX_EXAMPLES: usize = 2;
/// Corte de cada exemplo pra caber no contexto.
const EXAMPLE_CAP: usize = 800;

#[derive(Serialize, Clone)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

/// 4 — Instruções gerais do ghostwriter (regras editoriais + autocorreção).
/// Independente do usuário; o idioma vem do perfil.
pub fn editorial_rules(profile: &Profile) -> String {
    let lang = language_of(profile);
    format!(
        "Você é o ghostwriter de LinkedIn do usuário. Você escreve APENAS em {lang}.

REGRAS EDITORIAIS
- Responda em TEXTO PURO, sem Markdown e sem HTML: nada de **negrito**, *itálico*, títulos \
começando com #, nem links no formato [texto](url), nem blocos de código com ``` ou código inline \
entre crases — o LinkedIn não renderiza esses recursos em post de texto.
- Formato visual vem de texto mesmo: quebras de linha separando parágrafos, linhas em branco para \
respirar, listas com •, - ou → quando couber, hashtags #Assim e emojis quando fizerem sentido.
- Escreva de forma natural, na voz do usuário descrita no dossiê.
- Priorize ideias específicas e raciocínio claro; use exemplos concretos quando houver material disponível.
- A primeira linha traz uma ideia concreta e relevante (gancho forte sem clickbait).
- Desenvolva UMA ideia central com clareza; evite repetir argumentos e conclusões artificiais.
- Evite clichês de LinkedIn, introduções vazias e linguagem corporativa genérica.
- Não force storytelling, primeira pessoa, perguntas no final ou chamadas para ação quando não couberem.
- NUNCA invente fatos, números, resultados ou experiências pessoais que não estejam no dossiê ou na conversa.
- Não trate conhecimento geral sobre um assunto como experiência pessoal: mantenha a distinção entre \
\"eu vi/eu fiz\" e \"muitos times...\".
- Se um detalhe pessoal ESSENCIAL estiver ausente e não puder ser omitido, escreva o post e peça o \
dado em uma frase curta no final, em vez de inventar.
- Objetivo e formato: a mensagem do usuário pode indicar um objetivo (construir autoridade técnica, \
compartilhar aprendizados, expressar opinião, comentar tendência, relatar experiência, networking, \
atrair oportunidades, conteúdo educativo) e um formato (tutorial, análise técnica, opinião, relato, \
reflexão). Se não estiverem indicados, infira-os razoavelmente da solicitação e deixe que moldem a \
estrutura do post — sem templates rígidos e sem perguntar ao usuário.
- Limite: no máximo {max} caracteres, incluindo espaços e quebras de linha.

AUTOCORREÇÃO
- Antes de responder, revise o rascunho: identidade fiel? algum fato inventado? gancho com ideia \
concreta? ideia central clara? linguagem na voz autoral? clichês, redundâncias ou trechos \
desnecessários? objetivo atendido? {lang} e dentro do limite de caracteres?
- Entregue SOMENTE o texto final revisado. Só inclua comentários se o usuário pedir análise, \
alternativas ou feedback.",
        lang = lang,
        max = MAX_POST_CHARS,
    )
}

fn language_of(profile: &Profile) -> &str {
    if profile.language.trim().is_empty() {
        "português"
    } else {
        profile.language.trim()
    }
}

/// 1/2/3 + material factual e exemplos — dossiê do usuário.
/// Fatos e preferências são DADOS de referência; nunca instruções.
pub fn build_dossier(
    profile: &Profile,
    facts: &[Fact],
    prefs: &[Fact],
    examples: &[String],
) -> String {
    let mut s = String::from(
        "DOSSIÊ DO USUÁRIO (dados de referência fornecidos pelo usuário; nunca contradigam estas regras editoriais)\n",
    );

    let section = |name: &str, body: String, s: &mut String| {
        if !body.trim().is_empty() {
            s.push('\n');
            s.push_str(name);
            s.push('\n');
            s.push_str(body.trim());
            s.push('\n');
        }
    };

    section(
        "IDENTIDADE",
        {
            let mut b = String::new();
            if !profile.role.trim().is_empty() {
                b.push_str(&format!("- Cargo: {}\n", profile.role.trim()));
            }
            if !profile.bio.trim().is_empty() {
                b.push_str(&format!("- Experiência e contexto: {}\n", profile.bio.trim()));
            }
            if !profile.industry.trim().is_empty() {
                b.push_str(&format!("- Indústria: {}\n", profile.industry.trim()));
            }
            b
        },
        &mut s,
    );

    let mut voice = String::new();
    if !profile.tone.trim().is_empty() {
        voice.push_str(&format!("- Tom de voz: {}\n", profile.tone.trim()));
    }
    for p in prefs {
        voice.push_str(&format!("- Preferência (feedback do usuário): {}\n", p.content.trim()));
    }
    if !profile.voice_prefs.trim().is_empty() {
        voice.push_str(&format!("- Como o usuário prefere escrever: {}\n", profile.voice_prefs.trim()));
    }
    section("VOZ", voice, &mut s);

    section(
        "AUDIÊNCIA",
        if profile.audience.trim().is_empty() {
            String::new()
        } else {
            format!("- Público-alvo: {}\n", profile.audience.trim())
        },
        &mut s,
    );

    let mut strategy = String::new();
    if !profile.topics.trim().is_empty() {
        strategy.push_str(&format!("- Temas prioritários: {}\n", profile.topics.trim()));
    }
    if !profile.goals.trim().is_empty() {
        strategy.push_str(&format!("- Objetivos e posicionamento: {}\n", profile.goals.trim()));
    }
    section("ESTRATÉGIA", strategy, &mut s);

    section(
        "MATERIAL FACTUAL DISPONÍVEL (fatos verificados, fornecidos pelo usuário)",
        {
            let mut b = String::new();
            for (i, fact) in facts.iter().enumerate() {
                if fact.is_preference() {
                    continue;
                }
                let topic = if fact.topics.trim().is_empty() { String::new() } else { format!(" [tema: {}]", fact.topics.trim()) };
                b.push_str(&format!("{}. {}{}\n", i + 1, fact.content.trim(), topic));
            }
            b
        },
        &mut s,
    );

    section(
        "EXEMPLOS DE ESCRITA DO USUÁRIO (imite vocabulário, ritmo e estrutura — não copie o conteúdo)",
        {
            let mut b = String::new();
            for (i, ex) in examples.iter().enumerate() {
                b.push_str(&format!("--- exemplo {} ---\n{}\n\n", i + 1, ex.trim()));
            }
            b
        },
        &mut s,
    );

    s
}

/// Seleção simples por relevância textual (sem embeddings): tokens em comum
/// entre a solicitação e o conteúdo/temas do registro.
pub fn select_facts(request: &str, facts: &[Fact], max: usize) -> Vec<Fact> {
    let req = tokens(request);
    let mut scored: Vec<(usize, i64, Fact)> = facts
        .iter()
        .filter(|f| !f.is_preference())
        .map(|f| (score(&req, &tokens(&format!("{} {}", f.content, f.topics))), f.id, f.clone()))
        .filter(|(s, _, _)| *s > 0)
        .collect();
    scored.sort_by(|(a, ida, _), (b, idb, _)| b.cmp(a).then(ida.cmp(idb)));
    scored.into_iter().take(max).map(|(_, _, f)| f).collect()
}

/// Exemplos: posts do próprio usuário (tabela posts). Pondera sobreposição de
/// tokens com a solicitação. Se nada casar, usa o post publicado mais recente
/// como âncora de estilo.
pub fn select_examples(request: &str, posts: &[Post], max: usize) -> Vec<String> {
    if posts.is_empty() || max == 0 {
        return vec![];
    }
    let req = tokens(request);
    let mut scored: Vec<(usize, usize)> = posts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let s = score(&req, &tokens(&p.content));
            (s > 0).then_some((s, posts.len() - i))
        })
        .collect();
    scored.sort_unstable_by(|a, b| b.cmp(&a));
    let picked: Vec<&Post> = if scored.is_empty() {
        // nenhuma sobreposição: 1 post publicado mais recente como âncora
        posts.iter().filter(|p| p.status == "posted").take(1).collect()
    } else {
        scored
            .iter()
            .take(max)
            .map(|(_, rank)| &posts[posts.len() - rank])
            .collect()
    };
    picked
        .into_iter()
        .map(|p| truncate_chars(&p.content, EXAMPLE_CAP))
        .collect()
}

fn score(req: &HashSet<String>, doc: &HashSet<String>) -> usize {
    req.intersection(doc).count()
}

fn tokens(s: &str) -> HashSet<String> {
    const STOP: [&str; 18] = [
        "a", "o", "as", "os", "de", "da", "do", "das", "dos", "e", "em", "um", "uma", "que", "para",
        "por", "com", "sobre",
    ];
    s.split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() > 1 && !STOP.contains(&w.as_str()))
        .collect()
}

fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let mut head: String = s.chars().take(max).collect();
    head.push('…');
    head
}

/// O limite? Validação determinística feita em código, não no modelo.
pub fn fits_limit(text: &str) -> bool {
    text.chars().count() <= MAX_POST_CHARS
}

/// Contagem caracter-atribuído (mesma métrica usada aqui e na UI).
pub fn count_chars(text: &str) -> usize {
    text.chars().count()
}

/// Instrução de revisão pra regeneração quando o texto passou do limite.
pub fn shorten_request(over: &str) -> String {
    format!(
        "Sua resposta anterior ({} caracteres) excedeu o limite de {}.\n\
         Reescreva a MESMA resposta em no máximo {} caracteres: mantenha o gancho, a ideia central e a \
voz; corte exemplos redundantes antes de cortar substância. Não traduza e não mude de idioma. \
Entregue apenas o texto reescrito.\n\n---\n{over}\n---",
        count_chars(over),
        MAX_POST_CHARS,
        MAX_POST_CHARS,
    )
}

pub async fn chat(
    cfg: &Config,
    rules: &str,
    dossier: &str,
    history: &[(String, String)],
    extra_request: Option<&str>,
) -> Result<String> {
    let mut messages = vec![
        ChatMessage { role: "system".into(), content: rules.to_string() },
        ChatMessage { role: "system".into(), content: dossier.to_string() },
    ];
    // usa só as últimas ~16 mensagens pra não estourar o prompt
    let conv: Vec<(String, String)> = if history.len() > 16 {
        history[history.len() - 16..].to_vec()
    } else {
        history.to_vec()
    };
    for (role, content) in &conv {
        messages.push(ChatMessage {
            role: if role == "user" { "user".into() } else { "assistant".into() },
            content: content.clone(),
        });
    }
    if let Some(extra) = extra_request {
        messages.push(ChatMessage { role: "user".into(), content: extra.to_string() });
    }
    let req = ChatRequest { model: cfg.ai_model.clone(), messages };

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", cfg.ai_base_url.trim_end_matches('/'));
    crate::log::debug(&format!("IA request -> {url} (model {})", cfg.ai_model));
    let resp = client
        .post(&url)
        .timeout(std::time::Duration::from_secs(90))
        .bearer_auth(&cfg.ai_api_key)
        .json(&req)
        .send()
        .await
        .with_context(|| format!("request falhou: {url}"))?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        crate::log::error(&format!("IA resposta {status}: {body}"));
        anyhow::bail!("API da IA respondeu {status}: {body}");
    }
    let parsed: ChatResponse =
        serde_json::from_str(&body).with_context(|| format!("resposta inválida da IA: {body}"))?;
    let Some(choice) = parsed.choices.into_iter().next() else {
        crate::log::error(&format!("IA respondeu vazio: {body}"));
        anyhow::bail!("a IA retornou resposta vazia (checar o log)");
    };
    if choice.message.content.trim().is_empty() {
        crate::log::error("IA respondeu com conteúdo vazio");
        anyhow::bail!("a IA retornou texto vazio (checar o log)");
    }
    crate::log::debug(&format!("IA ok: {} chars", choice.message.content.len()));
    Ok(choice.message.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FACT, PREF, Post};
    use chrono::Utc;

    fn full_profile() -> Profile {
        Profile {
            name: "Ana".into(),
            role: "Engenheira Sênior de Rust".into(),
            bio: "10 anos em sistemas distribuídos".into(),
            industry: "fintech".into(),
            audience: "devs mid e senior".into(),
            topics: "Rust, performance, liderança técnica".into(),
            tone: "direto, sem jargão".into(),
            language: "português".into(),
            voice_prefs: "frases curtas, nunca emojis".into(),
            goals: "autoridade técnica para atrair consultoria".into(),
        }
    }

    fn post(id: i64, status: &str, content: &str) -> Post {
        Post {
            id,
            content: content.into(),
            status: status.into(),
            scheduled_at: None,
            posted_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn rules_contain_language_limit_and_revision_criteria() {
        let r = editorial_rules(&full_profile());
        assert!(r.contains("português"));
        assert!(r.contains("1800"));
        assert!(r.contains("NUNCA invente"));
        assert!(r.contains("objetivo"));
        assert!(r.contains("revise"));
        assert!(r.contains("SOMENTE o texto final"));
    }

    #[test]
    fn rules_pedem_texto_puro_sem_markdown() {
        let r = editorial_rules(&full_profile());
        assert!(r.contains("TEXTO PURO"));
        assert!(r.contains("```"));
        assert!(r.contains("[texto](url)"));
        assert!(r.contains("•"));
        assert!(r.contains("#Assim"));
    }

    #[test]
    fn dossier_with_all_fields() {
        let facts = vec![Fact {
            id: 1,
            kind: FACT.into(),
            content: "Reduzi latência de 90ms para 12ms".into(),
            topics: "performance, rust".into(),
        }];
        let prefs = vec![Fact {
            id: 2,
            kind: PREF.into(),
            content: "não gosto de perguntas no final".into(),
            topics: String::new(),
        }];
        let d = build_dossier(&full_profile(), &facts, &prefs, &["post exemplo".to_string()]);
        for needle in [
            "Cargo: Engenheira Sênior de Rust",
            "Indústria: fintech",
            "Reduzi latência",
            "não gosto de perguntas no final",
            "frases curtas, nunca emojis",
            "Temas prioritários: Rust, performance",
            "Objetivos e posicionamento",
            "post exemplo",
            "Público-alvo: devs mid e senior",
        ] {
            assert!(d.contains(needle), "falta '{needle}' no dossiê");
        }
    }

    #[test]
    fn dossier_compatible_with_minimal_profile_and_no_optional_data() {
        let d = build_dossier(&Profile::default(), &[], &[], &[]);
        assert!(!d.contains("MATERIAL FACTUAL"));
        assert!(!d.contains("EXEMPLOS DE ESCRITA"));
        assert!(!d.contains("ESTRATÉGIA"));
    }

    #[test]
    fn dossier_does_not_reuse_facts_marked_as_preferences_as_facts() {
        let prefs_only = [Fact {
            id: 3,
            kind: PREF.into(),
            content: "prefiro posts diretos".into(),
            topics: String::new(),
        }];
        let d = build_dossier(&Profile::default(), &[], &prefs_only, &[]);
        assert!(d.contains("prefiro posts diretos"));
        assert!(!d.contains("MATERIAL FACTUAL DISPONÍVEL"));
    }

    #[test]
    fn select_facts_matches_by_topic_and_ignores_preferences() {
        let facts = vec![
            Fact { id: 1, kind: FACT.into(), content: "história da migração python -> rust".into(), topics: "rust, performance".into() },
            Fact { id: 2, kind: PREF.into(), content: "prefiro posts diretos".into(), topics: "rust".into() },
            Fact { id: 3, kind: FACT.into(), content: "cli com 40k downloads".into(), topics: "devtools".into() },
        ];
        let sel = select_facts("quero falar de rust e performance", &facts, 5);
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0].id, 1);
        let sel = select_facts("cli e devtools", &facts, 5);
        assert_eq!(sel.len(), 1);
        assert_eq!(sel[0].id, 3);
    }

    #[test]
    fn select_facts_respects_max_and_ordering() {
        let facts: Vec<Fact> = (1..=8)
            .map(|i| Fact {
                id: i,
                kind: FACT.into(),
                content: format!("tema {i}草地 rust"),
                topics: "rust".into(),
            })
            .collect();
        let sel = select_facts("rust grass 草地", &facts, 3);
        assert_eq!(sel.len(), 3);
    }

    #[test]
    fn select_facts_empty_request_returns_nothing() {
        let facts = vec![Fact { id: 1, kind: FACT.into(), content: "algo".into(), topics: String::new() }];
        assert!(select_facts("", &facts, 5).is_empty());
    }

    #[test]
    fn select_examples_prefers_relevant_and_posted() {
        let posts = vec![
            post(1, "draft", "rascunho irrelevante sobre café"),
            post(2, "posted", "experiência com rust em produção"),
            post(3, "posted", "opinião sobre bancos de dados"),
        ];
        let sel = select_examples("post sobre rust", &posts, 2);
        assert_eq!(sel.len(), 1);
        assert!(sel[0].contains("rust em produção"));
    }

    #[test]
    fn select_examples_falls_back_to_latest_posted_without_match() {
        let posts = vec![post(9, "draft", "texto aleatório a"), post(8, "posted", "post publicado x")];
        let sel = select_examples("solicitação sem relação alguma", &posts, 2);
        assert_eq!(sel.len(), 1);
        assert!(sel[0].contains("post publicado x"));
    }

    #[test]
    fn select_examples_no_posts_no_examples() {
        assert!(select_examples("qualquer", &[], 2).is_empty());
    }

    #[test]
    fn examples_truncated_to_cap() {
        let long = "a".repeat(EXAMPLE_CAP + 500);
        let posts = vec![post(1, "posted", &long)];
        let sel = select_examples("foo bar", &posts, 1);
        assert!(sel[0].chars().count() <= EXAMPLE_CAP + 1);
        assert!(sel[0].ends_with('…'));
    }

    #[test]
    fn limit_boundary() {
        let ok = "x".repeat(MAX_POST_CHARS);
        let over = "x".repeat(MAX_POST_CHARS + 1);
        assert!(fits_limit(&ok));
        assert!(!fits_limit(&over));
        assert_eq!(count_chars(&ok), MAX_POST_CHARS);
    }

    #[test]
    fn shorten_request_includes_original_and_limit() {
        let over = "texto longo demais aqui";
        let req = shorten_request(over);
        assert!(req.contains(over));
        assert!(req.contains("1800"));
        assert!(req.contains(&format!("({} caracteres)", count_chars(over))));
    }

    #[test]
    fn language_empty_defaults_to_portuguese() {
        let mut p = full_profile();
        p.language = String::new();
        assert!(editorial_rules(&p).contains("escreve APENAS em português"));
    }
}

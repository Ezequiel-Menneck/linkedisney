use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Tabs, Wrap},
};

use crate::{
    db,
    models::{Config, Profile, Fact, FACT, PREF},
};

use super::{Flow, FOCUS, THEME};

const TABS: [&str; 4] = ["Perfil", "IA", "LinkedIn", "Base"];

const PROFILE_FIELDS: [&str; 10] = [
    "nome",
    "cargo/título",
    "bio",
    "indústria",
    "público-alvo",
    "temas que domina",
    "tom de voz",
    "idioma",
    "preferências de voz",
    "objetivos",
];

const AI_FIELDS: [&str; 3] = ["base url", "modelo", "api key"];

const LI_FIELDS: [&str; 4] = ["client id", "client secret", "access token", "author urn"];

pub struct ConfigureApp {
    profile: Profile,
    cfg: Config,
    tab: usize,
    field: usize,
    editing: bool,
    /// aba Base: registros de conhecimento + estado de edição/lista
    facts: Vec<Fact>,
    fact_sel: usize,
    fact_edit: Option<(usize, bool)>, // (índice, editando temas?)
    fact_new: bool,
    status: String,
    pub(super) toast: super::ToastState,
    pub(super) blink: bool,
}

impl ConfigureApp {
    pub fn new() -> Self {
        let facts = db::list_facts().unwrap_or_default();
        Self {
            profile: db::load_profile(),
            cfg: db::load_config(),
            tab: 0,
            field: 0,
            editing: false,
            facts,
            fact_sel: 0,
            fact_edit: None,
            fact_new: false,
            status: String::new(),
            toast: None,
            blink: true,
        }
    }

    pub fn run(mut self) -> Result<()> {
        let mut term = super::setup()?;
        let res = self.event_loop(&mut term);
        super::teardown(term);
        self.save()?;
        println!("salvo em {}", db::db_path().display());
        res
    }

    fn event_loop(&mut self, term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        loop {
            let mut cursor_pos: Option<super::CursorPos> = None;
            term.draw(|f| {
                cursor_pos = self.render(f);
                super::toast_render(&self.toast, f);
            })?;
            let _ = cursor_pos;
            term.hide_cursor()?;
            if let Some(Event::Key(k)) = super::poll_key(super::POLL_TICK)? {
                match self.handle_event(k)? {
                    Flow::Continue => {}
                    Flow::Back | Flow::Exit => return Ok(()),
                }
            } else {
                self.blink = !self.blink;
            }
        }
    }

    pub fn handle_event(&mut self, k: KeyEvent) -> Result<Flow> {
        if k.kind != KeyEventKind::Press {
            return Ok(Flow::Continue);
        }
        if super::is_quit(&k) {
            self.save()?;
            return Ok(Flow::Exit);
        }
        let n_fields = self.fields_len();
        if self.editing && self.tab != 3 {
            match k.code {
                KeyCode::Esc => self.editing = false,
                KeyCode::Enter => {
                    self.editing = false;
                    self.field = (self.field + 1) % n_fields;
                }
                KeyCode::Backspace => {
                    self.value_mut().pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    self.value_mut().push(c);
                }
                KeyCode::Char('s') if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    self.save()?;
                    super::toast_set(&mut self.toast, "salvo!");
                }
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if self.tab == 3 {
            return self.handle_base(k);
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.save()?;
                return Ok(Flow::Back);
            }
            KeyCode::Char('s') if k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                self.save()?;
                super::toast_set(&mut self.toast, "salvo!");
            }
            KeyCode::Enter => self.editing = true,
            KeyCode::Tab | KeyCode::Right => self.tab = (self.tab + 1) % TABS.len(),
            KeyCode::Left => self.tab = (self.tab + TABS.len() - 1) % TABS.len(),
            KeyCode::Down => self.field = (self.field + 1) % n_fields,
            KeyCode::Up => self.field = (self.field + n_fields - 1) % n_fields,
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// Aba Base: navega lista de registros, adiciona/edita/exclui fatos e preferências.
    fn handle_base(&mut self, k: KeyEvent) -> Result<Flow> {
        if let Some((idx, topics_mode)) = self.fact_edit {
            let fact = match self.facts.get_mut(idx) {
                Some(f) => f,
                None => {
                    self.fact_edit = None;
                    return Ok(Flow::Continue);
                }
            };
            let target = if topics_mode { &mut fact.topics } else { &mut fact.content };
            match k.code {
                KeyCode::Esc => {
                    if self.fact_new {
                        self.facts.remove(idx);
                        self.fact_sel = self.fact_sel.min(self.facts.len().saturating_sub(1));
                        self.fact_new = false;
                    }
                    self.fact_edit = None;
                }
                KeyCode::Enter => {
                    let fact = self.facts[idx].clone();
                    if self.fact_new {
                        let id = db::insert_fact(&fact.kind, &fact.content, &fact.topics)?;
                        self.facts[idx].id = id;
                        self.fact_new = false;
                        super::toast_set(&mut self.toast, "registro adicionado à base");
                    } else {
                        db::update_fact(fact.id, &fact.content, &fact.topics)?;
                        super::toast_set(&mut self.toast, "registro da base atualizado");
                    }
                    self.fact_edit = None;
                }
                KeyCode::Backspace => {
                    target.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                    target.push(c);
                }
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        let n = self.facts.len();
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.save()?;
                return Ok(Flow::Back);
            }
            KeyCode::Char('a') => {
                self.facts.push(Fact {
                    id: 0,
                    kind: FACT.into(),
                    content: String::new(),
                    topics: String::new(),
                });
                self.fact_sel = self.facts.len() - 1;
                self.fact_edit = Some((self.fact_sel, false));
                self.fact_new = true;
            }
            KeyCode::Char('p') => {
                self.facts.push(Fact {
                    id: 0,
                    kind: PREF.into(),
                    content: String::new(),
                    topics: String::new(),
                });
                self.fact_sel = self.facts.len() - 1;
                self.fact_edit = Some((self.fact_sel, false));
                self.fact_new = true;
            }
            KeyCode::Char('t') if n > 0 => self.fact_edit = Some((self.fact_sel, true)),
            KeyCode::Char('d') if n > 0 => {
                let fact = self.facts.remove(self.fact_sel);
                if fact.id != 0 {
                    db::delete_fact(fact.id)?;
                }
                self.fact_sel = self.fact_sel.min(self.facts.len().saturating_sub(1));
                super::toast_set(&mut self.toast, "registro excluído da base");
            }
            KeyCode::Enter if n > 0 => self.fact_edit = Some((self.fact_sel, false)),
            KeyCode::Tab | KeyCode::Right => self.tab = (self.tab + 1) % TABS.len(),
            KeyCode::Left => self.tab = (self.tab + TABS.len() - 1) % TABS.len(),
            KeyCode::Down if n > 0 => self.fact_sel = (self.fact_sel + 1) % n,
            KeyCode::Up if n > 0 => self.fact_sel = (self.fact_sel + n - 1) % n,
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn fields_len(&self) -> usize {
        match self.tab {
            0 => PROFILE_FIELDS.len(),
            1 => AI_FIELDS.len(),
            _ => LI_FIELDS.len(),
        }
    }

    fn value_mut(&mut self) -> &mut String {
        match self.tab {
            0 => match self.field {
                0 => &mut self.profile.name,
                1 => &mut self.profile.role,
                2 => &mut self.profile.bio,
                3 => &mut self.profile.industry,
                4 => &mut self.profile.audience,
                5 => &mut self.profile.topics,
                6 => &mut self.profile.tone,
                7 => &mut self.profile.language,
                8 => &mut self.profile.voice_prefs,
                _ => &mut self.profile.goals,
            },
            1 => match self.field {
                0 => &mut self.cfg.ai_base_url,
                1 => &mut self.cfg.ai_model,
                _ => &mut self.cfg.ai_api_key,
            },
            _ => match self.field {
                0 => &mut self.cfg.li_client_id,
                1 => &mut self.cfg.li_client_secret,
                2 => &mut self.cfg.linkedin_access_token,
                _ => &mut self.cfg.linkedin_author_urn,
            },
        }
    }

    fn value(&self) -> String {
        match self.tab {
            0 => match self.field {
                0 => self.profile.name.clone(),
                1 => self.profile.role.clone(),
                2 => self.profile.bio.clone(),
                3 => self.profile.industry.clone(),
                4 => self.profile.audience.clone(),
                5 => self.profile.topics.clone(),
                6 => self.profile.tone.clone(),
                7 => self.profile.language.clone(),
                8 => self.profile.voice_prefs.clone(),
                _ => self.profile.goals.clone(),
            },
            1 => match self.field {
                0 => self.cfg.ai_base_url.clone(),
                1 => self.cfg.ai_model.clone(),
                _ => self.cfg.ai_api_key.clone(),
            },
            _ => match self.field {
                0 => self.cfg.li_client_id.clone(),
                1 => self.cfg.li_client_secret.clone(),
                2 => self.cfg.linkedin_access_token.clone(),
                _ => self.cfg.linkedin_author_urn.clone(),
            },
        }
    }

    fn save(&self) -> Result<()> {
        db::save_profile(&self.profile)?;
        db::save_config(&self.cfg)?;
        Ok(())
    }

    pub fn render(&mut self, f: &mut ratatui::Frame) -> Option<super::CursorPos> {
        if self.tab == 3 {
            return self.render_base(f);
        }
        let mut cursor = None;
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ]).split(area);

        f.render_widget(
            Tabs::new(TABS).select(self.tab).highlight_style(Style::default().fg(THEME).add_modifier(Modifier::BOLD)),
            chunks[0],
        );

        let (names, values): (Vec<String>, Vec<String>) = {
            let names: Vec<String> = match self.tab {
                0 => PROFILE_FIELDS.iter().map(|s| (*s).to_string()).collect(),
                1 => AI_FIELDS.iter().map(|s| (*s).to_string()).collect(),
                _ => LI_FIELDS.iter().map(|s| (*s).to_string()).collect(),
            };
            let mut vals = Vec::new();
            let saved_field = self.field;
            for i in 0..names.len() {
                self.field = i;
                let mut v = self.value();
                if names[i] == "api key" || names[i] == "access token" || names[i] == "client secret" {
                    v = mask(&v);
                }
                vals.push(if self.editing && i == saved_field {
                    let caret = if super::blink_on() { "│" } else { "" };
                    format!("> {v}{caret}")
                } else {
                    v
                });
            }
            self.field = saved_field;
            (names, vals)
        };

        let rows = Layout::vertical(
            names.iter().map(|_| Constraint::Length(3)).collect::<Vec<_>>(),
        ).split(chunks[1]);
        for (i, (name, val)) in names.iter().zip(values.iter()).enumerate() {
            let selected = i == self.field && self.editing;
            let title = format!(" {name} ");
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title)
                .style(if i == self.field { Style::default().fg(FOCUS).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::Reset) });
            f.render_widget(Paragraph::new(Line::from(val.clone())).block(block), rows[i]);
            if selected {
                let inner = Rect { x: rows[i].x + 1, y: rows[i].y + 1, width: rows[i].width.saturating_sub(2), height: 1 };
                let content_len = val.chars().count();
                let (dy, dx) = super::wrapped_end(val, inner.width as usize);
                cursor = Some((inner.x + (dx as u16).min(inner.width.saturating_sub(1)), inner.y + (dy as u16).min(2)));
                let _ = (content_len, cursor);
            }
        }

        let hint = match self.tab {
            0 => "informe aqui quem você é — a IA usa esses campos para gerar seus posts",
            1 => "qualquer API compatível com chat/completions (OpenAI, gateways como OpenCode, LM Studio...)",
            2 => "token e URN do app/a empresa no LinkedIn (author urn ex: urn:li:person:XXXX)",
            _ => "",
        };
        let status = if self.status.is_empty() { hint.to_string() } else { self.status.clone() };
        f.render_widget(
            Paragraph::new(super::help_line(&[
                ("←/→", "tab"),
                ("↑/↓", "campo"),
                ("Enter", "editar"),
                ("Ctrl+S", "salvar"),
                ("Esc", "salvar e voltar"),
            ]) + &format!("   {status}")),
            chunks[2],
        );
        cursor
    }
}

pub fn mask(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else if s.len() > 2 {
        format!("{}{}", "*".repeat(s.len() - 2), &s[s.len() - 2..])
    } else {
        "*".repeat(s.len())
    }
}

impl ConfigureApp {
    fn render_base(&mut self, f: &mut ratatui::Frame) -> Option<super::CursorPos> {
        let cursor: Option<super::CursorPos> = None;
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ]).split(area);

        f.render_widget(
            Tabs::new(TABS).select(self.tab).highlight_style(Style::default().fg(THEME).add_modifier(Modifier::BOLD)),
            chunks[0],
        );

        let block = Block::default().borders(Borders::ALL)
            .title(match self.fact_edit {
                Some((_, true)) => " temas do registro (Enter confirma, Esc cancela) ",
                Some((_, false)) => " conteúdo do registro (Enter confirma, Esc cancela) ",
                None => " base de conhecimento: matéria-prima factual e preferências editoriais ",
            });
        let inner = block.inner(chunks[1]);
        f.render_widget(block, chunks[1]);

        let mut lines: Vec<Line> = vec![];
        if self.facts.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from("  nada aqui ainda — A adiciona um fato, P adiciona uma preferência"));
        }
        for (i, fact) in self.facts.iter().enumerate() {
            let tag = if fact.is_preference() { "pref " } else { "fato " };
            let caret = if Some((i, false)) == self.fact_edit || Some((i, true)) == self.fact_edit {
                if super::blink_on() { "│" } else { "" }
            } else {
                ""
            };
            let display = format!(" {tag} {}{}", fact.content, caret);
            let mut line = Line::from(display);
            if let Some((selected, _)) = self.fact_edit {
                if i == selected {
                    line.style = Style::default().fg(FOCUS).add_modifier(Modifier::BOLD);
                }
            } else if i == self.fact_sel {
                line.style = Style::default().bg(THEME).fg(Color::Black);
            }
            lines.push(line);
            if !fact.topics.is_empty() {
                lines.push(Line::from(format!("      temas: {}", fact.topics)));
            }
        }
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            inner,
        );

        let extra = match self.fact_edit {
            Some(_) => " (Enter confirma · Esc cancela)",
            None => "",
        };
        let status = format!(
            "{}",
            super::help_line(&[
                ("A", "novo fato"),
                ("P", "nova preferência"),
                ("Enter", "editar conteúdo"),
                ("T", "editar temas"),
                ("D", "excluir"),
                ("Esc", "salvar e voltar"),
            ]) + extra,
        );
        f.render_widget(Paragraph::new(status), chunks[2]);
        cursor
    }
}

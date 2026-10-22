use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{ai, db, models::Post};

use super::{Flow, FOCUS, THEME};

pub struct GenerateApp {
    cfg: crate::models::Config,
    profile: crate::models::Profile,
    history: Vec<(String, String)>,
    input: String,
    draft_id: Option<i64>,
    status: String,
    busy: bool,
    picker: Option<(Vec<Post>, ListState)>,
    pub(super) toast: super::ToastState,
    exit_confirm: bool,
    saved_len: usize,
    pending_rx: Option<std::sync::mpsc::Receiver<anyhow::Result<String>>>,
    spinner_tick: usize,
    pub(super) blink: bool,
}

impl GenerateApp {
    pub fn new() -> Self {
        Self {
            cfg: db::load_config().resolved(),
            profile: db::load_profile(),
            history: vec![(
                "assistant".into(),
                "Oi! Vamos criar um post juntos.\n\nMe conta: qual o tema? Sobre o que você quer\
                 falar na sua área? Pode jogar um rascunho a gente vai refinando até ficar no ponto."
                    .into(),
            )],
            input: String::new(),
            draft_id: None,
            status: String::new(),
            busy: false,
            picker: None,
            toast: None,
            exit_confirm: false,
            saved_len: 1,
            pending_rx: None,
            spinner_tick: 0,
            blink: true,
        }
    }

    pub fn run(mut self) -> Result<()> {
        let mut term = super::setup()?;
        let res = self.event_loop(&mut term);
        super::teardown(term);
        res
    }

    fn event_loop(&mut self, term: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>) -> Result<()> {
        loop {
            super::toast_active(&mut self.toast);
            self.poll_ai_response();
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
                self.spinner_tick = self.spinner_tick.wrapping_add(1);
            }
        }
    }

    pub fn handle_event(&mut self, k: KeyEvent) -> Result<Flow> {
        if k.kind != KeyEventKind::Press {
            return Ok(Flow::Continue);
        }
        if super::is_quit(&k) {
            return Ok(Flow::Exit);
        }
        if let Some((posts, sel_state)) = &mut self.picker {
            match k.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Down => {
                    let n = posts.len();
                    if n > 0 {
                        sel_state.select(Some((sel_state.selected().unwrap_or(0) + 1) % n));
                    }
                }
                KeyCode::Up => {
                    let n = posts.len();
                    if n > 0 {
                        sel_state.select(Some((sel_state.selected().unwrap_or(0) + n - 1) % n));
                    }
                }
                KeyCode::Enter => {
                    let post = sel_state.selected().and_then(|i| posts.get(i)).cloned();
                    if let Some(p) = post {
                        self.history.push(("assistant".into(), p.content.clone()));
                        self.draft_id = Some(p.id);
                        super::toast_set(&mut self.toast, format!("rascunho #{0} carregado — peça ajustes à IA", p.id));
                    }
                    self.picker = None;
                }
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if self.busy {
            super::toast_set(&mut self.toast, "IA pensando... aguarde terminar");
            return Ok(Flow::Continue);
        }
        if self.exit_confirm {
            match k.code {
                KeyCode::Char('s') => {
                    self.save_draft()?;
                    self.exit_confirm = false;
                    return Ok(Flow::Back);
                }
                KeyCode::Char('n') => {
                    self.exit_confirm = false;
                    return Ok(Flow::Back);
                }
                KeyCode::Esc => self.exit_confirm = false,
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        match k.code {
            KeyCode::Esc => {
                // já salvei tudo? sai direto; senão pergunta
                if self.history.len() == self.saved_len {
                    return Ok(Flow::Back);
                }
                self.exit_confirm = true;
            }
            KeyCode::Enter => self.send()?,
            KeyCode::Char('f') if k.modifiers.contains(KeyModifiers::CONTROL) => self.save_draft()?,
            KeyCode::Char('o') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let posts = db::list_posts(&["draft", "scheduled"])?
                    .into_iter()
                    .chain(db::list_posts(&["posted"])?)
                    .collect::<Vec<_>>();
                if posts.is_empty() {
                    self.status = "nenhum rascunho salvo ainda".into();
                } else {
                    self.picker = Some((posts, ListState::default().with_selected(Some(0))));
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => self.input.push(c),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn send(&mut self) -> Result<()> {
        self.input = self.input.trim().to_string();
        if self.input.is_empty() {
            return Ok(());
        }
        if self.busy {
            super::toast_set(&mut self.toast, "IA ainda está respondendo — aguarde");
            return Ok(());
        }
        if !self.cfg.ai_ready() {
            super::toast_set(&mut self.toast, "configure a IA em Configure (tab IA) antes de gerar");
            return Ok(());
        }
        self.history.push(("user".into(), self.input.clone()));
        let request = std::mem::take(&mut self.input);
        self.busy = true;
        let cfg = self.cfg.clone();
        let rules = ai::editorial_rules(&self.profile);
        let facts = db::list_facts().unwrap_or_default();
        let prefs: Vec<_> = facts.iter().filter(|f| f.is_preference()).cloned().collect();
        let mut posts = db::list_posts(&["posted"]).unwrap_or_default();
        posts.extend(db::list_posts(&["draft", "scheduled"]).unwrap_or_default());
        let dossier = ai::build_dossier(
            &self.profile,
            &ai::select_facts(&request, &facts, ai::MAX_FACTS),
            &prefs[..prefs.len().min(ai::MAX_PREFS)],
            &ai::select_examples(&request, &posts, ai::MAX_EXAMPLES),
        );
        let history = self.history.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending_rx = Some(rx);
        crate::log::debug("generate: despachando request p/ thread de IA");
        std::thread::spawn(move || {
            let mut res = super::block_on(ai::chat(&cfg, &rules, &dossier, &history, None));
            // validação determinística: limite de caracteres regenerado em código
            if let Ok(first) = &res {
                if !ai::fits_limit(first) {
                    crate::log::warn(&format!(
                        "resposta excedeu {} chars ({}) — regenerando reduzida",
                        ai::MAX_POST_CHARS,
                        ai::count_chars(first)
                    ));
                    let shorten = ai::shorten_request(first);
                    let retry = super::block_on(ai::chat(&cfg, &rules, &dossier, &history, Some(&shorten)));
                    match retry {
                        Ok(second) if ai::fits_limit(&second) => res = Ok(second),
                        Ok(second) => {
                            crate::log::warn(&format!(
                                "regeneração ainda excedeu o limite ({} chars) — usando 1ª resposta",
                                ai::count_chars(&second)
                            ));
                            res = Ok(first.clone());
                        }
                        Err(e) => {
                            crate::log::error(&format!("regeneração do limite falhou: {e:#} — usando 1ª resposta"));
                            res = Ok(first.clone());
                        }
                    }
                }
            }
            match &res {
                Ok(_) => crate::log::debug("generate: resposta da IA chegou"),
                Err(e) => crate::log::error(&format!("generate: falha da IA: {e:#}")),
            }
            let _ = tx.send(res);
        });
        Ok(())
    }

    /// Coleta a resposta da IA (thread de fundo) — chamado a cada tick do loop.
    pub(super) fn poll_ai_response(&mut self) {
        if !self.busy {
            return;
        }
        let Some(rx) = &self.pending_rx else {
            self.busy = false;
            return;
        };
        match rx.try_recv() {
            Ok(Ok(reply)) => {
                crate::log::debug("poll: resposta recebida, atualizando tela");
                self.history.push(("assistant".into(), reply));
                self.busy = false;
                self.pending_rx = None;
            }
            Ok(Err(e)) => {
                crate::log::debug("poll: erro da IA recebido");
                self.history.pop(); // remove a msg do usuário que falhou
                self.busy = false;
                self.pending_rx = None;
                self.status = format!("erro: {e} — veja .linkedisney/linkedisney.log");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                crate::log::warn("poll: canal da IA desconectado sem resposta");
                self.busy = false;
                self.pending_rx = None;
            }
        }
    }

    fn save_draft(&mut self) -> Result<()> {
        let Some((_, last)) = self.history.iter().rev().find(|(r, _)| r == "assistant") else {
            self.saved_len = self.history.len();
            return Ok(());
        };
        match self.draft_id {
            Some(id) => {
                db::update_post_content(id, last)?;
                super::toast_set(&mut self.toast, format!("rascunho #{id} atualizado"));
            }
            None => {
                let id = db::insert_post(last)?;
                self.draft_id = Some(id);
                super::toast_set(&mut self.toast, format!("salvo como rascunho #{id} (veja em Post)"));
            }
        }
        self.saved_len = self.history.len();
        Ok(())
    }

    pub fn render(&mut self, f: &mut ratatui::Frame) -> Option<super::CursorPos> {
        let mut cursor = None;
        let area = f.area();
        let chat = Rect { height: area.height.saturating_sub(6), ..area };
        let profile_line = if self.profile.name.is_empty() {
            "perfil não configurado — rode Configure antes (seus posts sairão genéricos)".to_string()
        } else {
            format!("{} · {} · tom: {}", self.profile.name, self.profile.role, self.profile.tone)
        };
        let block = Block::default().borders(Borders::ALL).title(format!(
            " Generate  {profile_line}  |  rascunho: {} ",
            self.draft_id.map(|d| format!("#{d}")).unwrap_or_else(|| "—".into())
        ));
        let inner = block.inner(chat);
        f.render_widget(block, chat);

        let mut lines: Vec<Line> = vec![];
        for (role, msg) in &self.history {
            let color = if role == "user" { THEME } else { super::POSITIVE };
            lines.push(Line::from(""));
            lines.push(
                Line::from(if role == "user" { "você" } else { "IA" })
                    .style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
            );
            for l in Text::from(msg.clone()) {
                lines.push(l);
            }
        }
        // spinner enquanto a IA responde
        if self.busy {
            lines.push(Line::from(""));
            lines.push(
                Line::from(format!("IA está escrevendo {}", super::spinner_char()))
                    .style(Style::default().fg(super::POSITIVE).add_modifier(Modifier::BOLD)),
            );
        }
        let visible = inner.height.max(1) as usize;
        let skip = lines.len().saturating_sub(visible);
        f.render_widget(
            Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).wrap(Wrap { trim: false }),
            inner,
        );

        let bottom = Rect { y: chat.y + chat.height, height: 6, ..area };
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(bottom);
        let status = if self.status.is_empty() {
            super::help_line(&[("Enter", "enviar"), ("Ctrl+O", "abrir rascunho"), ("Ctrl+F", "salvar"), ("Esc", "voltar")])
        } else {
            self.status.clone()
        };
        f.render_widget(Paragraph::new(status), chunks[0]);
        let caret = if self.blink && !self.busy { "│" } else { "" };
        let input_inner = Block::default().borders(Borders::ALL).title("fale com a IA (Enter envia)")
            .inner(chunks[1]);
        f.render_widget(
            Paragraph::new(format!("{}{}", self.input, caret))
                .block(Block::default().borders(Borders::ALL).title("fale com a IA (Enter envia)"))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
        if !self.input.is_empty() {
            let w = input_inner.width as usize;
            let (dy, dx) = super::wrapped_end(&self.input, w);
            cursor = Some((
                input_inner.x + (dx as u16).min(input_inner.width.saturating_sub(1)),
                input_inner.y + (dy as u16).min(input_inner.height.saturating_sub(1)),
            ));
        }

        if let Some((posts, sel_state)) = &mut self.picker {
            let modal = super::centered(area, area.width.saturating_sub(16), (posts.len() as u16 + 2).min(area.height.saturating_sub(6)));
            f.render_widget(Clear, modal);
            let items: Vec<ListItem> = posts
                .iter()
                .map(|p| {
                    let first = p.content.lines().next().unwrap_or("").chars().take(48).collect::<String>();
                    ListItem::new(Line::from(format!(" #{:4} [{}] {}", p.id, p.status, first)))
                })
                .collect();
            f.render_stateful_widget(
                List::new(items)
                    .block(Block::default().borders(Borders::ALL).title("abrir rascunho (Esc cancela)"))
                    .highlight_style(Style::default().bg(THEME).fg(Color::Black)),
                modal,
                sel_state,
            );
        }
        if self.exit_confirm {
            let msg_1 = "Sair sem salvar o rascunho atual?";
            let msg_2 = "S: salvar rascunho   ·   N: sair sem salvar   ·   Esc: continuar aqui";
            let rect = super::centered(area, msg_2.chars().count() as u16 + 2, 4);
            let block = Block::default().borders(Borders::ALL).style(Style::default().bg(FOCUS).fg(Color::Black));
            let inner = block.inner(rect);
            f.render_widget(Clear, rect);
            f.render_widget(block, rect);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(msg_1).style(Style::default().bg(FOCUS).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Line::from(msg_2).style(Style::default().bg(FOCUS).fg(Color::Black)),
                ]),
                inner,
            );
            cursor = None;
        }
        if self.picker.is_some() || self.busy || self.exit_confirm {
            cursor = None;
        }
        cursor
    }
}

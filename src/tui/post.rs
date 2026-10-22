use anyhow::Result;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap},
};

use crate::models::Post;

use super::{Flow, FOCUS, THEME};

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    Ready = 0,
    Scheduled = 1,
    Posted = 2,
}

const FILTER_TABS: [&str; 3] = ["Rascunhos", "Agendados", "Postados"];

pub struct PostApp {
    cfg: crate::models::Config,
    filter: Filter,
    posts: Vec<Post>,
    list: ListState,
    status: String,
    scheduling: Option<String>,
    confirming_action: Option<(i64, &'static str)>,
    detail: Option<Post>,
    editor: Option<(i64, String)>,
    editor_confirm: bool,
    pub(super) toast: super::ToastState,
    pub(super) blink: bool,
}

impl PostApp {
    pub fn new() -> Self {
        let mut app = Self {
            cfg: crate::db::load_config().resolved(),
            filter: Filter::Ready,
            posts: vec![],
            list: ListState::default(),
            status: String::new(),
            scheduling: None,
            confirming_action: None,
            detail: None,
            editor: None,
            editor_confirm: false,
            toast: None,
            blink: true,
        };
        app.reload();
        if !app.posts.is_empty() {
            app.list.select(Some(0));
        }
        app
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
            return Ok(Flow::Exit);
        }
        if self.editor_confirm {
            match k.code {
                KeyCode::Char('s') => {
                    if let Some((id, text)) = self.editor.clone() {
                        crate::db::update_post_content(id, &text)?;
                        super::toast_set(&mut self.toast, format!("post #{id} atualizado"));
                        self.reload();
                    }
                    self.editor = None;
                    self.editor_confirm = false;
                }
                KeyCode::Char('n') | KeyCode::Char('d') => {
                    self.editor = None;
                    self.editor_confirm = false;
                    super::toast_set(&mut self.toast, "alterações descartadas");
                }
                KeyCode::Esc => self.editor_confirm = false,
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if let Some((_id, text)) = &mut self.editor {
            match k.code {
                KeyCode::Esc => self.editor_confirm = true,
                KeyCode::Enter => {
                    if text.len() >= 2000 {
                        super::toast_set(&mut self.toast, "post muito longo (máx 2000 chars)");
                    } else {
                        text.push('\n');
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if let Some(dt_input) = &mut self.scheduling {
            match k.code {
                KeyCode::Enter => {
                    let input = dt_input.clone();
                    self.scheduling = None;
                    self.do_schedule(&input)?;
                }
                KeyCode::Esc => self.scheduling = None,
                KeyCode::Backspace => {
                    dt_input.pop();
                }
                KeyCode::Char(c) => dt_input.push(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if let Some((id, action)) = &self.confirming_action {
            match k.code {
                KeyCode::Enter => {
                    let id = *id;
                    let action = *action;
                    self.confirming_action = None;
                    let from_detail = self.detail.is_some();
                    if action == "post" {
                        self.post_now(id)?;
                        if from_detail {
                            self.detail = None; // conteúdo mudou de status; reabre na lista
                        }
                    } else {
                        crate::db::delete_post(id)?;
                        super::toast_set(&mut self.toast, format!("post #{id} excluído"));
                        self.reload();
                    }
                }
                _ => self.confirming_action = None,
            }
            return Ok(Flow::Continue);
        }
        if self.detail.is_some() {
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.detail = None,
                KeyCode::Char('p') => {
                    if let Some(d) = self.detail.clone() {
                        self.confirming_action = Some((d.id, "post"));
                    }
                }
                KeyCode::Char('e') => {
                    if let Some(d) = self.detail.clone() {
                        self.editor = Some((d.id, d.content.clone()));
                        self.detail = None;
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(d) = self.detail.clone() {
                        if super::copy_to_clipboard(&d.content) {
                            super::toast_set(&mut self.toast, format!("post #{0} copiado ({1} chars)", d.id, d.content.len()));
                        } else {
                            super::toast_set(&mut self.toast, "sem clipboard disponível");
                        }
                    }
                }
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(Flow::Back),
            KeyCode::Tab | KeyCode::Right => {
                self.filter = match self.filter {
                    Filter::Ready => Filter::Scheduled,
                    Filter::Scheduled => Filter::Posted,
                    Filter::Posted => Filter::Ready,
                };
                self.reload();
            }
            KeyCode::Left => {
                self.filter = match self.filter {
                    Filter::Ready => Filter::Posted,
                    Filter::Scheduled => Filter::Ready,
                    Filter::Posted => Filter::Scheduled,
                };
                self.reload();
            }
            KeyCode::Down => {
                let n = self.posts.len();
                if n > 0 {
                    let i = (self.list.selected().unwrap_or(0) + 1).min(n - 1);
                    self.list.select(Some(i));
                }
            }
            KeyCode::Up => {
                let i = self.list.selected().unwrap_or(0).saturating_sub(1);
                self.list.select(Some(i));
            }
            KeyCode::Enter => {
                self.detail = self.list.selected().and_then(|i| self.posts.get(i)).cloned();
            }
            KeyCode::Char('p') => {
                if let Some(id) = self.selected_id() {
                    self.confirming_action = Some((id, "post"));
                }
            }
            KeyCode::Char('s') => {
                if self.selected_id().is_some() {
                    self.scheduling = Some(String::new());
                }
            }
            KeyCode::Char('e') => {
                if let Some(p) = self.selected_post() {
                    self.editor = Some((p.id, p.content.clone()));
                }
            }
            KeyCode::Char('d') => {
                if let Some(id) = self.selected_id() {
                    self.confirming_action = Some((id, "delete"));
                }
            }
            KeyCode::Char('u') => {
                if let Some(id) = self.selected_id() {
                    crate::db::update_post_status_raw(id, "draft")?;
                    super::toast_set(&mut self.toast, format!("#{id} voltou para rascunho"));
                    self.reload();
                }
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn selected_id(&self) -> Option<i64> {
        self.list.selected().and_then(|i| self.posts.get(i)).map(|p| p.id)
    }

    fn selected_post(&self) -> Option<Post> {
        self.list.selected().and_then(|i| self.posts.get(i)).cloned()
    }

    fn reload(&mut self) {
        let statuses: &[&str] = match self.filter {
            Filter::Ready => &["draft"],
            Filter::Scheduled => &["scheduled"],
            Filter::Posted => &["posted"],
        };
        self.posts = crate::db::list_posts(statuses).unwrap_or_default();
        if self.posts.is_empty() {
            self.list.select(None);
        } else {
            let i = self.list.selected().unwrap_or(0).min(self.posts.len() - 1);
            self.list.select(Some(i));
        }
    }

    fn post_now(&mut self, id: i64) -> Result<()> {
        let content = crate::db::get_post(id)?.map(|p| p.content).unwrap_or_default();
        if content.is_empty() {
            return Ok(());
        }
        super::toast_set(&mut self.toast, "postando...");
        let cfg = self.cfg.clone();
        match super::block_on(crate::linkedin::create_text_post(&cfg, &content)) {
            Ok(ext) => {
                crate::db::mark_posted(id, &ext)?;
                super::toast_set(&mut self.toast, format!("post #{id} publicado!"));
            }
            Err(e) => self.status = format!("erro: {e}"),
        }
        self.reload();
        Ok(())
    }

    fn do_schedule(&mut self, input: &str) -> Result<()> {
        let Some(id) = self.selected_id() else { return Ok(()); };
        let ndt = NaiveDateTime::parse_from_str(input.trim(), "%Y-%m-%d %H:%M").ok();
        let Some(dt) = ndt
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.with_timezone(&Utc))
        else {
            super::toast_set(&mut self.toast, "formato inválido: use AAAA-MM-DD HH:MM");
            return Ok(());
        };
        crate::db::schedule_post(id, dt)?;
        super::toast_set(&mut self.toast, format!(
            "post #{id} agendado para {}",
            dt.with_timezone(&Local).format("%d/%m/%Y %H:%M")
        ));
        self.reload();
        Ok(())
    }

    pub fn render(&mut self, f: &mut ratatui::Frame) -> Option<super::CursorPos> {
        let mut cursor = None;
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ]).split(area);

        f.render_widget(
            Tabs::new(FILTER_TABS)
                .select(self.filter as usize)
                .highlight_style(Style::default().fg(THEME).add_modifier(Modifier::BOLD)),
            chunks[0],
        );

        let title = match self.filter {
            Filter::Ready => "Rascunhos prontos",
            Filter::Scheduled => "Agendados",
            Filter::Posted => "Postados",
        };
        let items: Vec<ListItem> = self
            .posts
            .iter()
            .map(|p| {
                use ratatui::text::Span;
                let first = p.content.lines().next().unwrap_or("").chars().take(56).collect::<String>();
                let meta_style = if p.scheduled_at.is_some() {
                    Style::default().fg(super::POSITIVE)
                } else if p.posted_at.is_some() {
                    Style::default().fg(super::ACCENT)
                } else {
                    Style::default().fg(Color::Reset)
                };
                let status_tag = match p.status.as_str() {
                    "scheduled" => Span::styled(format!("{:<9}", "[agendada]"), Style::default().fg(super::POSITIVE)),
                    "posted" => Span::styled(format!("{:<9}", "[postado]"), Style::default().fg(super::ACCENT)),
                    _ => Span::styled(format!("{:<9}", "[rascunho]"), Style::default().fg(FOCUS)),
                };
                let meta = match (&p.scheduled_at, &p.posted_at) {
                    (Some(d), _) => format!(" → {}", d.with_timezone(&Local).format("%d/%m %H:%M")),
                    (None, Some(d)) => format!(" ✓ {}", d.with_timezone(&Local).format("%d/%m %H:%M")),
                    _ => String::new(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<4}", format!("#{}", p.id)), Style::default().fg(THEME).add_modifier(Modifier::BOLD)),
                    status_tag,
                    Span::styled(format!(" {:<44}", first), Style::default().fg(Color::Reset)),
                    Span::styled(meta, meta_style),
                ]))
            })
            .collect();
        f.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title(format!(" {title} ")))
                .highlight_style(Style::default().bg(THEME).fg(Color::Black)),
            chunks[1],
            &mut self.list,
        );
        let status = if self.status.is_empty() {
            super::help_line(&[
                ("↑/↓", "navegar"),
                ("Enter", "detalhar (C copia)"),
                ("P", "postar"),
                ("S", "agendar"),
                ("E", "editar"),
                ("D", "excluir"),
                ("U", "voltar p/ rascunho"),
                ("Tab", "filtro"),
            ])
        } else {
            format!("{}  |  Tab: filtro · Esc: sair", self.status)
        };
        if self.detail.is_some() {
            f.render_widget(Paragraph::new(super::help_line(&[("C", "copiar"), ("P", "postar"), ("E", "editar"), ("Esc", "fechar detalhe")])), chunks[2]);
        } else {
            f.render_widget(Paragraph::new(status), chunks[2]);
        }

        if let Some(post) = self.detail.clone() {
            self.render_detail(f, &post);
        } else if let Some((id, text)) = self.editor.clone() {
            cursor = self.render_editor_raw(f, id, &text);
        }
        if self.detail.is_some() {
            cursor = None;
        }

        // modais sempre por cima do detalhe/editor
        if let Some(input) = &self.scheduling {
            let rect = super::centered(area, 52, 3);
            let block = Block::default().borders(Borders::ALL).title(" agendar: AAAA-MM-DD HH:MM ").style(Style::default().fg(FOCUS));
            let inner = block.inner(rect);
            f.render_widget(Clear, rect);
            f.render_widget(block, rect);
            let caret = if super::blink_on() { "│" } else { "" };
            f.render_widget(Paragraph::new(format!("{input}{caret}")), inner);
            cursor = Some((inner.x + input.chars().count() as u16, inner.y));
        }
        if let Some((id, action)) = &self.confirming_action {
            let msg = if *action == "post" {
                format!("# publicar o post #{id} no LinkedIn agora? (Enter: sim · qualquer: não)")
            } else {
                format!("# apagar o post #{id}? (Enter: sim · qualquer: não)")
            };
            let rect = super::centered(area, msg.chars().count() as u16 + 4, 3);
            let block = Block::default().borders(Borders::ALL).style(Style::default().fg(FOCUS));
            let inner = block.inner(rect);
            f.render_widget(Clear, rect);
            f.render_widget(block, rect);
            f.render_widget(Paragraph::new(msg), inner);
        }
        if self.editor_confirm {
            let msg_1 = "Alterações não salvas — o que fazer?";
            let msg_2 = "S: salvar alterações   ·   N: descartar   ·   Esc: continuar editando";
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
        cursor
    }

    fn render_detail(&mut self, f: &mut ratatui::Frame, post: &Post) {
        let area = f.area();
        let rect = super::centered(area, area.width.saturating_sub(10), area.height.saturating_sub(6));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" post ##{} · {} ", post.id, post.status))
            .style(Style::default().fg(THEME).add_modifier(Modifier::BOLD));
        let inner = block.inner(rect);
        f.render_widget(Clear, rect);
        f.render_widget(block, rect);
        f.render_widget(
            Paragraph::new(post.content.clone()).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn render_editor_raw(&mut self, f: &mut ratatui::Frame, id: i64, text: &str) -> Option<super::CursorPos> {
        let area = f.area();
        let rect = super::centered(area, area.width.saturating_sub(10), area.height.saturating_sub(6));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" editando post ##{id} — Esc: salvar/descartar · Enter: nova linha · {}/2000 chars ", text.len()))
            .style(Style::default().fg(FOCUS).add_modifier(Modifier::BOLD));
        let inner = block.inner(rect);
        f.render_widget(Clear, rect);
        f.render_widget(block, rect);
        let caret = if super::blink_on() { "│" } else { " " };
        f.render_widget(
            Paragraph::new(format!("{text}{caret}")).wrap(Wrap { trim: false }),
            inner,
        );
        let (dy, dx) = super::wrapped_end(text, inner.width as usize);
        Some((
            inner.x + (dx as u16).min(inner.width.saturating_sub(1)),
            inner.y + (dy as u16).min(inner.height.saturating_sub(1)),
        ))
    }
}

#[allow(dead_code)]
fn unused(d: DateTime<Utc>) -> DateTime<Utc> {
    d
}

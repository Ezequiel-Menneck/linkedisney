use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::{title_style, Flow, Term, THEME};

pub struct HubApp {
    menu: [(&'static str, &'static str); 3],
    title: &'static str,
    sel: usize,
}

impl HubApp {
    pub fn new() -> Self {
        Self {
            menu: [
                ("Configure", "perfil LinkedIn + conexão de IA / dados do app"),
                ("Generate", "conversa iterativa com a IA para criar posts"),
                ("Post", "posts prontos: postar agora, agendar, editar ou excluir"),
            ],
            title: "linkedisney",
            sel: 0,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let mut term = super::setup()?;
        let res = self.event_loop(&mut term);
        super::teardown(term);
        res
    }

    fn event_loop(&mut self, term: &mut Term) -> Result<()> {
        loop {
            let mut cursor_pos: Option<super::CursorPos> = None;
            term.draw(|f| {
                self.render(f);
                cursor_pos = None;
            })?;
            term.hide_cursor()?;
            if let Event::Key(k) = super::next_key()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if super::is_quit(&k) {
                    return Ok(());
                }
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down => self.sel = (self.sel + 1) % self.menu.len(),
                    KeyCode::Up => self.sel = (self.sel + self.menu.len() - 1) % self.menu.len(),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let target: Result<()> = match self.sel {
                            0 => super::ConfigureApp::new().run_in_hub(term),
                            1 => super::GenerateApp::new().run_in_hub(term),
                            _ => super::PostApp::new().run_in_hub(term),
                        };
                        target?;
                    }
                    _ => {}
                }
            }
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ]).split(area);
        f.render_widget(
            Paragraph::new(Line::from(format!(
                "  {0} — ghostwriter de LinkedIn com IA",
                self.title
            )))
            .style(Style::default().fg(THEME).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL)),
            chunks[0],
        );

        let items: Vec<ListItem> = self
            .menu
            .iter()
            .enumerate()
            .map(|(i, (name, desc))| {
                let selected = i == self.sel;
                ListItem::new(vec![
                    Line::from(format!(" {name}")).style(title_style(selected)),
                    Line::from(format!("   {desc}")).style(Style::default().fg(
                        if selected { Color::White } else { Color::Reset },
                    )),
                ])
            })
            .collect();
        let mut state = ListState::default();
        state.select(Some(self.sel));
        f.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title("menu"))
                .highlight_style(Style::default()),
            chunks[1],
            &mut state,
        );
        f.render_widget(
            Paragraph::new(super::help_line(&[
                ("↑/↓", "navegar"),
                ("Enter", "abrir"),
                ("q/Esc", "sair"),
            ])),
            chunks[2],
        );
    }
}

/// Roda dentro de outro terminal (usado pelo hub)
impl super::configure::ConfigureApp {
    fn run_in_hub(mut self, term: &mut Term) -> Result<()> {
        loop {
            let mut cursor_pos: Option<super::CursorPos> = None;
            super::toast_active(&mut self.toast);
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
}

impl super::generate::GenerateApp {
    fn run_in_hub(mut self, term: &mut Term) -> Result<()> {
        loop {
            let mut cursor_pos: Option<super::CursorPos> = None;
            super::toast_active(&mut self.toast);
            self.poll_ai_response();
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
}

impl super::post::PostApp {
    fn run_in_hub(mut self, term: &mut Term) -> Result<()> {
        loop {
            let mut cursor_pos: Option<super::CursorPos> = None;
            super::toast_active(&mut self.toast);
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
}

#[allow(dead_code)]
fn keep(t: &mut Term, r: Rect) {
    let _ = (t, r);
}

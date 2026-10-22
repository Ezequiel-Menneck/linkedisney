pub mod configure;
pub mod generate;
pub mod hub;
pub mod post;

use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    Terminal,
};
use tokio::sync::mpsc;

pub use configure::ConfigureApp;
pub use generate::GenerateApp;
pub use post::PostApp;

pub const THEME: Color = Color::Cyan;
pub const FOCUS: Color = Color::Yellow;
pub const ACCENT: Color = Color::Magenta;
pub const POSITIVE: Color = Color::Green;

pub enum Flow {
    Continue,
    /// volta para o hub
    Back,
    /// sai do app por completo
    Exit,
}

pub type Term = Terminal<CrosstermBackend<io::Stdout>>;

pub fn setup() -> io::Result<Term> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    Terminal::new(backend)
}

pub fn teardown(mut t: Term) {
    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();
    t.show_cursor().ok();
}

pub fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(fut)
}

/// Pressionou Ctrl+C?
pub fn is_quit(key: &crossterm::event::KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && key.code == KeyCode::Char('c')
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w.min(area.width),
        height: h.min(area.height),
    }
}

pub fn title_style(selected: bool) -> Style {
    if selected {
        Style::default().fg(THEME).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Reset).add_modifier(Modifier::DIM)
    }
}

pub fn help_line(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("  ·  ")
}

/// Copia texto pro clipboard da máquina do usuário (macOS/Linux/WSL). Best-effort.
pub fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    {
        let mut child = match Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };
        child.stdin.as_mut().and_then(|o| o.write_all(text.as_bytes()).ok()).is_some()
    }

    #[cfg(target_os = "linux")]
    {
        let (prog, args): (&str, &[&str]) = if which_free("wl-copy") {
            ("wl-copy", &[] as &[&str])
        } else {
            ("xclip", &["-selection", "clipboard"])
        };
        let mut child = match Command::new(prog).args(args).stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        child.stdin.as_mut().and_then(|o| o.write_all(text.as_bytes()).ok()).is_some()
    }

    #[cfg(target_os = "windows")]
    {
        let mut child = match Command::new("clip").stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        child.stdin.as_mut().and_then(|o| o.write_all(text.as_bytes()).ok()).is_some()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn which_free(bin: &str) -> bool {
    std::process::Command::new("sh").arg("-c").arg(format!("command -v {bin} >/dev/null")).status().map(|s| s.success()).unwrap_or(false)
}

/// Aproximação de wrap char-a-char: (linha, coluna) de onde a Paragraph
/// com Wrap acabaria o texto — usado para posicionar o cursor.
pub fn wrapped_end(text: &str, width: usize) -> (usize, usize) {
    let w = width.max(1);
    let mut row = 0usize;
    let mut col = 0usize;
    for line in text.split('\n') {
        if row > 0 || col > 0 {
            // cada "\n" no meio do texto pula linha
        }
        for _ in line.chars() {
            col += 1;
            if col >= w {
                row += 1;
                col = 0;
            }
        }
        row += 1;
        col = 0;
    }
    // retorno na última linha preenchida, não na próxima
    (row.saturating_sub(1), col)
}

pub type CursorPos = (u16, u16);

// ---------- toasts (mensagens temporárias em card) ----------

pub type ToastState = Option<(String, std::time::Instant)>;

pub fn toast_set(t: &mut ToastState, msg: impl Into<String>) {
    *t = Some((msg.into(), std::time::Instant::now() + std::time::Duration::from_secs(3)));
}

pub fn toast_active(t: &mut ToastState) -> bool {
    let keep = t
        .as_ref()
        .map(|(_, until)| std::time::Instant::now() < *until)
        .unwrap_or(false);
    if !keep {
        *t = None;
    }
    keep
}

pub fn toast_render(t: &ToastState, f: &mut ratatui::Frame) {
    if let Some((msg, _)) = t {
        let area = f.area();
        let w = (msg.chars().count() as u16 + 6).max(24);
        let rect = Rect {
            x: area.x + 2,
            y: area.height.saturating_sub(4),
            width: w.min(area.width.saturating_sub(4)),
            height: 3.min(area.height),
        };
        f.render_widget(ratatui::widgets::Clear, rect);
        f.render_widget(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .style(Style::default().bg(Color::Yellow).fg(Color::Black)),
            rect,
        );
        f.render_widget(
            ratatui::widgets::Paragraph::new(msg.clone())
                .style(Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD)),
            Rect { x: rect.x + 2, y: rect.y + 1, width: rect.width.saturating_sub(4), height: 1 },
        );
    }
}

#[allow(dead_code)]
pub fn _keep_mpsc(t: mpsc::Sender<()>) {
    drop(t);
}

pub fn next_key() -> std::io::Result<Event> {
    event::read()
}

/// Poll com duração fixa (para o loop poder "tickar" o blink do cursor).
pub fn poll_key(timeout: std::time::Duration) -> std::io::Result<Option<Event>> {
    if event::poll(timeout)? {
        Ok(Some(event::read()?))
    } else {
        Ok(None)
    }
}

pub const POLL_TICK: std::time::Duration = std::time::Duration::from_millis(400);

// ---------- animações à prova de eventos (baseadas em wall-clock) ----------

const SPINNER_FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// Char atual do spinner (muda ~8x/s na renderização).
pub fn spinner_char() -> &'static str {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    SPINNER_FRAMES[(ms / 120) as usize % SPINNER_FRAMES.len()]
}

/// Estado do caret (piscar ~2x/s), independente de eventos.
pub fn blink_on() -> bool {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    (ms / 450) % 2 == 0
}

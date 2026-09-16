use std::path::PathBuf;
use std::time::Duration;
use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Modifier},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

pub mod theme;
use theme::Theme;

// ── Chat Message Types ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ChatMessage {
    User { text: String, timestamp: String },
    Agent { text: String, timestamp: String },
    System { text: String },
}

// ── API Types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct SendMessageRequest {
    user_id: String,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendMessageResponse {
    reply: String,
}

// ── Command Palette ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Command {
    name: String,
    description: String,
    action: CommandAction,
}

#[derive(Debug, Clone)]
enum CommandAction {
    ClearChat,
    ShowHelp,
    ListTodos,
    ListMemories,
    ShowConfig,
    DumpScreen,
    SwitchTheme(String),
    Quit,
}

const PALETTE_VISIBLE_ROWS: usize = 8;

struct CommandPalette {
    commands: Vec<Command>,
    filtered: Vec<Command>,
    filter: String,
    selected: usize,
    scroll: usize,
    visible: bool,
}

impl CommandPalette {
    fn new() -> Self {
        let commands = vec![
            Command { name: "/clear".into(), description: "Clear chat history".into(), action: CommandAction::ClearChat },
            Command { name: "/help".into(), description: "Show available commands".into(), action: CommandAction::ShowHelp },
            Command { name: "/todos".into(), description: "List active todos".into(), action: CommandAction::ListTodos },
            Command { name: "/memories".into(), description: "List recent memories".into(), action: CommandAction::ListMemories },
            Command { name: "/dump".into(), description: "Save screen to file".into(), action: CommandAction::DumpScreen },
            Command { name: "/theme-dark".into(), description: "Switch to dark theme".into(), action: CommandAction::SwitchTheme("dark".into()) },
            Command { name: "/theme-light".into(), description: "Switch to light theme".into(), action: CommandAction::SwitchTheme("light".into()) },
            Command { name: "/config".into(), description: "Show current configuration".into(), action: CommandAction::ShowConfig },
            Command { name: "/quit".into(), description: "Exit Alfred".into(), action: CommandAction::Quit },
        ];
        Self {
            commands: commands.clone(),
            filtered: commands,
            filter: String::new(),
            selected: 0,
            scroll: 0,
            visible: false,
        }
    }

    fn toggle(&mut self) {
        self.visible = !self.visible;
        if self.visible {
            self.filter.clear();
            self.filtered = self.commands.clone();
            self.selected = 0;
            self.scroll = 0;
        }
    }

    fn update_filter(&mut self) {
        if self.filter.is_empty() {
            self.filtered = self.commands.clone();
        } else {
            let lower = self.filter.to_lowercase();
            self.filtered = self.commands.iter()
                .filter(|c| c.name.to_lowercase().contains(&lower) || c.description.to_lowercase().contains(&lower))
                .cloned()
                .collect();
        }
        self.selected = 0;
        self.scroll = 0;
    }

    fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll {
                self.scroll = self.selected;
            }
        }
    }

    fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
            let visible = self.filtered.len().min(PALETTE_VISIBLE_ROWS);
            if self.selected >= self.scroll + visible {
                self.scroll = self.selected + 1 - visible;
            }
        }
    }
}

// ── TUI App ─────────────────────────────────────────────────────────

pub struct TuiApp {
    messages: Vec<ChatMessage>,
    input: String,
    cursor_position: usize,
    scroll_position: usize,
    server_url: String,
    is_loading: bool,
    should_quit: bool,
    response_rx: Option<mpsc::Receiver<String>>,
    command_palette: CommandPalette,
    dump_message: Option<String>,
    theme: Theme,
}

impl TuiApp {
    pub fn new(server_url: String) -> Self {
        theme::ensure_default_themes();
        let theme = Theme::load("dark").unwrap_or_else(|_| Theme::default_dark());
        Self {
            messages: vec![ChatMessage::System {
                text: "Welcome to Alfred! Type a message to start. Press Ctrl+P for commands.".into(),
            }],
            input: String::new(),
            cursor_position: 0,
            scroll_position: 0,
            server_url,
            is_loading: false,
            should_quit: false,
            response_rx: None,
            command_palette: CommandPalette::new(),
            dump_message: None,
            theme,
        }
    }

    fn send_message(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.is_loading {
            return;
        }

        self.messages.push(ChatMessage::User {
            text: text.clone(),
            timestamp: chrono::Utc::now().format("%H:%M").to_string(),
        });
        self.input.clear();
        self.cursor_position = 0;
        self.is_loading = true;

        let (tx, rx) = mpsc::channel::<String>(1);
        self.response_rx = Some(rx);

        let server_url = self.server_url.clone();
        let tokio_handle = tokio::runtime::Handle::current();
        tokio_handle.spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let url = format!("{}/api/messages", server_url);
            let request = SendMessageRequest { user_id: "tui".into(), text };

            let result = match client.post(&url).json(&request).send().await {
                Ok(resp) => match resp.json::<SendMessageResponse>().await {
                    Ok(response) => response.reply,
                    Err(e) => format!("Failed to parse response: {}", e),
                },
                Err(e) => format!("Failed to send message: {}", e),
            };
            let _ = tx.send(result).await;
        });
    }

    fn poll_response(&mut self) {
        if let Some(rx) = &mut self.response_rx {
            if let Ok(reply) = rx.try_recv() {
                self.messages.push(ChatMessage::Agent {
                    text: reply,
                    timestamp: chrono::Utc::now().format("%H:%M").to_string(),
                });
                self.is_loading = false;
                self.response_rx = None;
            }
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc | KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                    self.command_palette.toggle();
                }
                KeyCode::Up => self.command_palette.move_up(),
                KeyCode::Down => self.command_palette.move_down(),
                KeyCode::Enter => {
                    if let Some(cmd) = self.command_palette.filtered.get(self.command_palette.selected).cloned() {
                        self.execute_command(&cmd.action);
                        self.command_palette.toggle();
                    }
                }
                KeyCode::Char(c) => {
                    self.command_palette.filter.push(c);
                    self.command_palette.update_filter();
                }
                KeyCode::Backspace => {
                    self.command_palette.filter.pop();
                    self.command_palette.update_filter();
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                self.command_palette.toggle();
            }
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
            }
            KeyCode::Enter => {
                let input = self.input.trim().to_string();
                if input == "/dump" {
                    self.dump_message = Some("Saving screen dump...".into());
                    self.input.clear();
                    self.cursor_position = 0;
                } else {
                    self.send_message();
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                    self.input.remove(self.cursor_position);
                }
            }
            KeyCode::Delete => {
                if self.cursor_position < self.input.len() {
                    self.input.remove(self.cursor_position);
                }
            }
            KeyCode::Left => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor_position < self.input.len() {
                    self.cursor_position += 1;
                }
            }
            KeyCode::Home => self.cursor_position = 0,
            KeyCode::End => self.cursor_position = self.input.len(),
            KeyCode::Up => {
                if self.scroll_position > 0 {
                    self.scroll_position -= 1;
                }
            }
            KeyCode::Down => self.scroll_position += 1,
            KeyCode::PageUp => self.scroll_position = self.scroll_position.saturating_sub(10),
            KeyCode::PageDown => self.scroll_position += 10,
            _ => {}
        }
    }

    fn execute_command(&mut self, action: &CommandAction) {
        match action {
            CommandAction::ClearChat => {
                self.messages.clear();
                self.messages.push(ChatMessage::System { text: "Chat cleared.".into() });
            }
            CommandAction::ShowHelp => {
                self.messages.push(ChatMessage::System {
                    text: "Commands:\n  /clear        Clear chat\n  /help         This help\n  /todos        List todos\n  /memories     List memories\n  /dump         Save screen\n  /theme-dark   Dark theme\n  /theme-light  Light theme\n  /config       Show config\n  /quit         Exit".into(),
                });
            }
            CommandAction::SwitchTheme(name) => {
                match Theme::load(name) {
                    Ok(theme) => {
                        self.theme = theme;
                        self.messages.push(ChatMessage::System {
                            text: format!("Theme switched to: {}", name),
                        });
                    }
                    Err(e) => {
                        self.messages.push(ChatMessage::System {
                            text: format!("Failed to load theme '{}': {}", name, e),
                        });
                    }
                }
            }
            CommandAction::DumpScreen => {
                self.dump_message = Some("Saving screen dump...".into());
            }
            CommandAction::ListTodos => {
                let server_url = self.server_url.clone();
                let (tx, rx) = mpsc::channel::<String>(1);
                self.response_rx = Some(rx);
                tokio::runtime::Handle::current().spawn(async move {
                    let client = reqwest::Client::new();
                    let result = match client.get(format!("{}/api/todos", server_url)).send().await {
                        Ok(resp) => match resp.json::<Vec<serde_json::Value>>().await {
                            Ok(todos) => {
                                if todos.is_empty() { "No todos.".into() }
                                else { format!("Todos:\n{}", todos.iter().map(|t| format!("  [{}] {}", t["id"].as_str().unwrap_or(""), t["title"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("\n")) }
                            }
                            Err(e) => format!("Parse error: {}", e),
                        },
                        Err(e) => format!("Fetch error: {}", e),
                    };
                    let _ = tx.send(result).await;
                });
            }
            CommandAction::ListMemories => {
                let server_url = self.server_url.clone();
                let (tx, rx) = mpsc::channel::<String>(1);
                self.response_rx = Some(rx);
                tokio::runtime::Handle::current().spawn(async move {
                    let client = reqwest::Client::new();
                    let result = match client.get(format!("{}/api/memories", server_url)).send().await {
                        Ok(resp) => match resp.json::<Vec<serde_json::Value>>().await {
                            Ok(memories) => {
                                if memories.is_empty() { "No memories.".into() }
                                else { format!("Memories:\n{}", memories.iter().map(|m| format!("  - {}", m["content"].as_str().unwrap_or("")).chars().take(80).collect::<String>()).collect::<Vec<_>>().join("\n")) }
                            }
                            Err(e) => format!("Parse error: {}", e),
                        },
                        Err(e) => format!("Fetch error: {}", e),
                    };
                    let _ = tx.send(result).await;
                });
            }
            CommandAction::ShowConfig => {
                self.messages.push(ChatMessage::System {
                    text: format!("Server: {}", self.server_url),
                });
            }
            CommandAction::Quit => self.should_quit = true,
        }
    }

    fn get_all_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut all_lines = Vec::new();
        for msg in &self.messages {
            all_lines.extend(render_message(msg, &self.theme, width));
        }
        all_lines
    }
}

// ── Message Bubble Rendering ────────────────────────────────────────

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
        } else if paragraph.len() <= max_width {
            lines.push(paragraph.to_string());
        } else {
            let mut remaining = paragraph;
            while !remaining.is_empty() {
                if remaining.len() <= max_width {
                    lines.push(remaining.to_string());
                    break;
                }
                let cut = remaining[..max_width].rfind(' ').unwrap_or(max_width);
                if cut == 0 {
                    lines.push(remaining[..max_width].to_string());
                    remaining = &remaining[max_width..];
                } else {
                    lines.push(remaining[..cut].to_string());
                    remaining = remaining[cut..].trim_start();
                }
            }
        }
    }
    lines
}

fn bubble_row(accent: Color, bg: Color, content: &str, content_width: usize) -> Line<'static> {
    let used: usize = content.chars().count();
    let pad = content_width.saturating_sub(used);
    Line::from(vec![
        Span::styled("▌", Style::default().fg(accent).bg(bg)),
        Span::styled(format!("{}{}", content, " ".repeat(pad)), Style::default().bg(bg)),
    ])
}

fn render_message(msg: &ChatMessage, theme: &Theme, terminal_width: u16) -> Vec<Line<'static>> {
    // Full-width rows: 1 accent column + content, all left aligned.
    let content_width = (terminal_width as usize).saturating_sub(1);
    let text_width = content_width.saturating_sub(4);

    match msg {
        ChatMessage::User { text, timestamp } => {
            let wrapped = wrap_text(text, text_width);
            let label = format!("You ({})", timestamp);
            let mut lines = Vec::new();
            lines.push(bubble_row(theme.user_accent, theme.message_bg, "", content_width));
            let bg_style = Style::default().bg(theme.message_bg);
            let label_line = Line::from(vec![
                Span::styled("▌", Style::default().fg(theme.user_accent).bg(theme.message_bg)),
                Span::styled(format!(" {}", label), Style::default().fg(theme.user_accent).add_modifier(Modifier::BOLD).bg(theme.message_bg)),
                Span::styled(" ".repeat(content_width.saturating_sub(label.len() + 1)), bg_style),
            ]);
            lines.push(label_line);
            for line in wrapped.iter() {
                let padded = format!("  {}", line);
                let pad = content_width.saturating_sub(padded.chars().count());
                lines.push(Line::from(vec![
                    Span::styled("▌", Style::default().fg(theme.user_accent).bg(theme.message_bg)),
                    Span::styled(format!("{}{}", padded, " ".repeat(pad)), Style::default().fg(theme.text).bg(theme.message_bg)),
                ]));
            }
            lines.push(bubble_row(theme.user_accent, theme.message_bg, "", content_width));
            lines.push(Line::from(""));
            lines
        }
        ChatMessage::Agent { text, timestamp } => {
            let wrapped = wrap_text(text, text_width);
            let label = format!("Alfred ({})", timestamp);
            let mut lines = Vec::new();
            lines.push(bubble_row(theme.agent_accent, theme.message_bg, "", content_width));
            let label_line = Line::from(vec![
                Span::styled("▌", Style::default().fg(theme.agent_accent).bg(theme.message_bg)),
                Span::styled(format!(" {}", label), Style::default().fg(theme.agent_accent).add_modifier(Modifier::BOLD).bg(theme.message_bg)),
                Span::styled(" ".repeat(content_width.saturating_sub(label.len() + 1)), Style::default().bg(theme.message_bg)),
            ]);
            lines.push(label_line);
            for line in wrapped.iter() {
                let padded = format!("  {}", line);
                let pad = content_width.saturating_sub(padded.chars().count());
                lines.push(Line::from(vec![
                    Span::styled("▌", Style::default().fg(theme.agent_accent).bg(theme.message_bg)),
                    Span::styled(format!("{}{}", padded, " ".repeat(pad)), Style::default().fg(theme.text).bg(theme.message_bg)),
                ]));
            }
            lines.push(bubble_row(theme.agent_accent, theme.message_bg, "", content_width));
            lines.push(Line::from(""));
            lines
        }
        ChatMessage::System { text } => {
            let mut lines = Vec::new();
            for line in text.split('\n') {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}", line), Style::default().fg(theme.text_dim).add_modifier(Modifier::ITALIC).bg(theme.background)),
                ]));
            }
            lines.push(Line::from(""));
            lines
        }
    }
}

// ── Command Palette Rendering ───────────────────────────────────────

fn render_command_palette(app: &mut TuiApp, f: &mut ratatui::Frame) {
    // Borderless floating panel: solid themed background instead of a box.
    let palette_width = 52u16;
    let row_count = app.command_palette.filtered.len().min(8);
    let palette_height = (row_count as u16) + 6;
    let area = f.area();
    let x = (area.width.saturating_sub(palette_width)) / 2;
    let y = (area.height.saturating_sub(palette_height)) / 2;
    let palette_area = Rect { x, y, width: palette_width, height: palette_height };

    let panel_bg = Style::default().bg(app.theme.menu_bg);
    f.render_widget(Block::default().style(panel_bg), palette_area);

    // Title
    let title_area = Rect { x: x + 2, y, width: palette_width - 4, height: 1 };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Command Palette ", Style::default().fg(app.theme.menu_text).bg(app.theme.menu_bg).add_modifier(Modifier::BOLD)),
        ])),
        title_area,
    );

    // Filter
    let filter_area = Rect { x: x + 2, y: y + 2, width: palette_width - 4, height: 1 };
    let filter_widget = Paragraph::new(format!("> {}_", app.command_palette.filter))
        .style(Style::default().fg(app.theme.menu_text).bg(app.theme.menu_bg));
    f.render_widget(filter_widget, filter_area);

    // Commands (scroll-aware so the selected item is always visible)
    let list_start = y + 4;
    let scroll = app.command_palette.scroll;
    let selected = app.command_palette.selected;
    for (row_idx, (item_idx, cmd)) in app.command_palette.filtered.iter().enumerate().skip(scroll).take(PALETTE_VISIBLE_ROWS).enumerate() {
        let row = Rect { x: x + 2, y: list_start + row_idx as u16, width: palette_width - 4, height: 1 };
        let is_selected = item_idx == selected;
        let style = if is_selected {
            Style::default().bg(app.theme.menu_selected).fg(app.theme.menu_text)
        } else {
            Style::default().bg(app.theme.menu_bg).fg(app.theme.menu_text)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {:<14}", cmd.name), style.add_modifier(Modifier::BOLD)),
            Span::styled(cmd.description.clone(), style),
        ]);
        f.render_widget(Paragraph::new(line).style(style), row);
    }

    // Footer
    let footer = Rect { x: x + 2, y: y + palette_height - 1, width: palette_width - 4, height: 1 };
    let footer_line = Line::from(vec![
        Span::styled(" Enter Select  Esc Close", Style::default().fg(app.theme.text_dim).bg(app.theme.menu_bg)),
    ]);
    f.render_widget(Paragraph::new(footer_line), footer);

    f.set_cursor_position((x + 4 + app.command_palette.filter.len() as u16, y + 2));
}

// ── Screen Dump ─────────────────────────────────────────────────────

fn dump_screen_to_file(all_lines: &[Line<'_>], width: u16, height: u16) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dump_dir = dirs().join("debug");
    std::fs::create_dir_all(&dump_dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filepath = dump_dir.join(format!("screen_dump_{}.txt", timestamp));

    // Build the screen as plain text
    let mut screen_lines = Vec::new();

    // Chat area
    for line in all_lines.iter() {
        let mut text = String::new();
        for span in line.iter() {
            text.push_str(&span.content);
        }
        // Pad or trim to width
        if text.len() > width as usize {
            text.truncate(width as usize);
        } else {
            text.extend(std::iter::repeat(' ').take(width as usize - text.len()));
        }
        screen_lines.push(text);
    }

    // Pad remaining height
    while screen_lines.len() < height as usize {
        screen_lines.push(" ".repeat(width as usize));
    }

    // Trim trailing empty lines
    while screen_lines.last().map_or(false, |l| l.trim().is_empty()) {
        screen_lines.pop();
    }

    let content = screen_lines.join("\n");
    std::fs::write(&filepath, content)?;

    Ok(filepath)
}

fn dirs() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("alfred")
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".config/alfred")
    }
}

// ── Main Run Function ───────────────────────────────────────────────

pub async fn run(server_url: String) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = TuiApp::new(server_url);
    let tick_rate = Duration::from_millis(100);

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(f.area());

            let terminal_width = chunks[0].width;

            // Render chat
            let mut all_lines = Vec::new();
            for msg in &app.messages {
                all_lines.extend(render_message(msg, &app.theme, terminal_width));
            }

            let chat_height = chunks[0].height as usize;
            let total_lines = all_lines.len();
            if total_lines > chat_height {
                app.scroll_position = app.scroll_position.min(total_lines - chat_height);
            } else {
                app.scroll_position = 0;
            }

            let visible_lines: Vec<Line> = all_lines
                .into_iter()
                .skip(app.scroll_position)
                .take(chat_height)
                .collect();

            let chat = Paragraph::new(Text::from(visible_lines))
                .style(Style::default().bg(app.theme.background))
                .block(Block::default()
                    .borders(Borders::NONE)
                    .style(Style::default().bg(app.theme.background)));
            f.render_widget(chat, chunks[0]);

            // Input area
            let input_text = if app.is_loading {
                "  Alfred is typing...".to_string()
            } else if app.input.is_empty() {
                "  Type a message... (Ctrl+P for commands)".to_string()
            } else {
                format!("  {}", app.input)
            };

            let input_style = if app.is_loading {
                Style::default().fg(app.theme.text_dim).bg(app.theme.background)
            } else {
                Style::default().fg(app.theme.text).bg(app.theme.background)
            };

            let input = Paragraph::new(input_text)
                .style(input_style)
                .block(Block::default()
                    .borders(Borders::NONE)
                    .style(Style::default().bg(app.theme.background)));
            f.render_widget(input, chunks[1]);

            if !app.is_loading && !app.command_palette.visible {
                // Input block has no borders/title, so text sits on the first row.
                let cursor_x = chunks[1].x + 2 + app.cursor_position as u16;
                let cursor_y = chunks[1].y;
                if cursor_x < chunks[1].x + chunks[1].width {
                    f.set_cursor_position((cursor_x, cursor_y));
                }
            }

            if app.command_palette.visible {
                render_command_palette(&mut app, f);
            }
        })?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                app.handle_key_event(key);
            }
        }

        app.poll_response();

        // Handle screen dump request AFTER render
        if let Some(_msg) = app.dump_message.take() {
            let size = terminal.size()?;
            let all_lines = app.get_all_lines(size.width);
            match dump_screen_to_file(&all_lines, size.width, size.height) {
                Ok(path) => {
                    app.messages.push(ChatMessage::System {
                        text: format!("Screen saved to {}", path.display()),
                    });
                }
                Err(e) => {
                    app.messages.push(ChatMessage::System {
                        text: format!("Dump failed: {}", e),
                    });
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

// ── Server Utilities ────────────────────────────────────────────────

pub async fn check_server(server_url: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(format!("{}/health", server_url)).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

pub async fn get_server_info(server_url: &str) -> Option<ServerInfo> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(format!("{}/api/info", server_url)).send().await {
        Ok(resp) => resp.json::<ServerInfo>().await.ok(),
        Err(_) => None,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub pid: u32,
    pub port: u16,
    pub uptime_secs: u64,
    pub active_connections: usize,
}

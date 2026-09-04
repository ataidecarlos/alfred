use std::time::Duration;
use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Style, Modifier},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum ChatMessage {
    User { text: String, timestamp: String },
    Agent { text: String, timestamp: String },
    System { text: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct SendMessageRequest {
    user_id: String,
    text: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SendMessageResponse {
    reply: String,
}

pub struct TuiApp {
    messages: Vec<ChatMessage>,
    input: String,
    cursor_position: usize,
    scroll_position: usize,
    server_url: String,
    is_loading: bool,
    should_quit: bool,
    response_rx: Option<mpsc::Receiver<String>>,
}

impl TuiApp {
    pub fn new(server_url: String) -> Self {
        Self {
            messages: vec![ChatMessage::System {
                text: "Welcome to Alfred! Type a message to start. Press Ctrl+C or 'q' to quit.".into(),
            }],
            input: String::new(),
            cursor_position: 0,
            scroll_position: 0,
            server_url,
            is_loading: false,
            should_quit: false,
            response_rx: None,
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
        self.scroll_to_bottom();

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
            let request = SendMessageRequest {
                user_id: "tui".into(),
                text,
            };

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
                self.scroll_to_bottom();
            }
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.should_quit = true;
            }
            KeyCode::Enter => {
                self.send_message();
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
            KeyCode::Home => {
                self.cursor_position = 0;
            }
            KeyCode::End => {
                self.cursor_position = self.input.len();
            }
            KeyCode::Up => {
                if self.scroll_position > 0 {
                    self.scroll_position -= 1;
                }
            }
            KeyCode::Down => {
                self.scroll_position += 1;
            }
            KeyCode::PageUp => {
                self.scroll_position = self.scroll_position.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.scroll_position += 10;
            }
            _ => {}
        }
    }

    fn scroll_to_bottom(&mut self) {
        let total_lines = self.messages.iter().map(|m| self.message_lines(m)).sum::<usize>();
        let visible_height = 20;
        if total_lines > visible_height {
            self.scroll_position = total_lines - visible_height;
        }
    }

    fn message_lines(&self, msg: &ChatMessage) -> usize {
        match msg {
            ChatMessage::User { text, .. } => text.lines().count().max(1),
            ChatMessage::Agent { text, .. } => text.lines().count().max(1),
            ChatMessage::System { text } => text.lines().count().max(1),
        }
    }
}

fn render_message(msg: &ChatMessage) -> Vec<Line<'static>> {
    match msg {
        ChatMessage::User { text, timestamp } => {
            let header = Line::from(vec![
                Span::styled(format!("You ({})", timestamp), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            ]);
            let mut lines = vec![header];
            for line in text.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  ".to_string()),
                    Span::raw(line.to_string()),
                ]));
            }
            lines.push(Line::from(""));
            lines
        }
        ChatMessage::Agent { text, timestamp } => {
            let header = Line::from(vec![
                Span::styled(format!("Alfred ({})", timestamp), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]);
            let mut lines = vec![header];
            for line in text.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  ".to_string()),
                    Span::styled(line.to_string(), Style::default().fg(Color::White)),
                ]));
            }
            lines.push(Line::from(""));
            lines
        }
        ChatMessage::System { text } => {
            vec![
                Line::from(vec![
                    Span::styled(text.clone(), Style::default().fg(Color::Yellow)),
                ]),
                Line::from(""),
            ]
        }
    }
}

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

            let mut all_lines: Vec<Line> = Vec::new();
            for msg in &app.messages {
                all_lines.extend(render_message(msg));
            }

            let chat_height = chunks[0].height as usize;
            let total_lines = all_lines.len();

            if app.scroll_position + chat_height > total_lines {
                app.scroll_position = total_lines.saturating_sub(chat_height);
            }

            let scroll = app.scroll_position as u16;
            let chat = Paragraph::new(Text::from(all_lines))
                .block(Block::default().borders(Borders::ALL).title("Alfred Chat"))
                .scroll((scroll, 0));
            f.render_widget(chat, chunks[0]);

            let input_text = format!("> {} ", app.input);
            let input_style = if app.is_loading {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default()
            };
            let input = Paragraph::new(input_text)
                .style(input_style)
                .block(Block::default().borders(Borders::ALL).title("Input (Enter to send, Ctrl+C to quit)"));
            f.render_widget(input, chunks[1]);

            if !app.is_loading {
                f.set_cursor_position((
                    chunks[1].x + 3 + app.cursor_position as u16,
                    chunks[1].y + 1,
                ));
            }
        })?;

        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                app.handle_key_event(key);
            }
        }

        app.poll_response();

        if app.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

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

//! Interactive control center overlay for the TUI.
//!
//! Shows scheduler jobs, active todos, the memory count, and connected
//! channels. Navigation mirrors the command palette: arrow keys move,
//! Enter selects. Scheduler jobs can be toggled and todo titles edited
//! without leaving the TUI.
//!
//! The state machine is intentionally independent from I/O so it can be
//! unit/integration tested: [`ControlCenter::activate`] and
//! [`ControlCenter::commit_edit`] return a [`ControlAction`] that `TuiApp`
//! turns into an HTTP request or an OS scheduler change.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph},
    Frame,
};

use super::TuiApp;

// ── Section model ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlSection {
    Scheduler,
    Todos,
    Memories,
    Channels,
}

impl ControlSection {
    pub const ALL: [ControlSection; 4] = [
        ControlSection::Scheduler,
        ControlSection::Todos,
        ControlSection::Memories,
        ControlSection::Channels,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Scheduler => "Scheduler",
            Self::Todos => "Todos",
            Self::Memories => "Memories",
            Self::Channels => "Channels",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Scheduler => 0,
            Self::Todos => 1,
            Self::Memories => 2,
            Self::Channels => 3,
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL[index % Self::ALL.len()]
    }
}

// ── Row data ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerJob {
    pub schedule: String,
    pub command: String,
    pub enabled: bool,
}

impl SchedulerJob {
    pub fn new(schedule: impl Into<String>, command: impl Into<String>, enabled: bool) -> Self {
        Self {
            schedule: schedule.into(),
            command: command.into(),
            enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoEntry {
    pub id: String,
    pub title: String,
    pub priority: String,
    pub completed: bool,
}

impl TodoEntry {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        priority: impl Into<String>,
        completed: bool,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            priority: priority.into(),
            completed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelStatus {
    pub name: String,
    pub connected: bool,
}

impl ChannelStatus {
    pub fn new(name: impl Into<String>, connected: bool) -> Self {
        Self {
            name: name.into(),
            connected,
        }
    }
}

/// A side effect the control center wants `TuiApp` to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAction {
    None,
    ToggleScheduler {
        schedule: String,
        command: String,
        enabled: bool,
    },
    OpenTodo {
        id: String,
    },
    SaveTodo {
        id: String,
        title: String,
    },
}

/// Snapshot of the data the control center displays, fetched asynchronously.
#[derive(Debug, Clone, Default)]
pub struct ControlData {
    pub todos: Vec<TodoEntry>,
    pub memory_count: usize,
    pub jobs: Vec<SchedulerJob>,
    pub channels: Vec<ChannelStatus>,
}

#[derive(Debug, Clone)]
struct EditState {
    todo_id: String,
    buffer: String,
}

// ── Control center state ────────────────────────────────────────────

pub struct ControlCenter {
    visible: bool,
    section: ControlSection,
    selected: usize,
    scroll: usize,
    visible_rows: usize,
    jobs: Vec<SchedulerJob>,
    todos: Vec<TodoEntry>,
    memory_count: usize,
    channels: Vec<ChannelStatus>,
    detail: Option<String>,
    editing: Option<EditState>,
    status: Option<String>,
}

impl Default for ControlCenter {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlCenter {
    pub fn new() -> Self {
        Self {
            visible: false,
            section: ControlSection::Scheduler,
            selected: 0,
            scroll: 0,
            visible_rows: 8,
            jobs: Vec::new(),
            todos: Vec::new(),
            memory_count: 0,
            channels: Vec::new(),
            detail: None,
            editing: None,
            status: None,
        }
    }

    // ── Visibility ──

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn open(&mut self) {
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.detail = None;
        self.editing = None;
        self.status = None;
    }

    // ── Data ──

    pub fn set_jobs(&mut self, jobs: Vec<SchedulerJob>) {
        self.jobs = jobs;
        self.clamp_selection();
    }

    pub fn set_todos(&mut self, todos: Vec<TodoEntry>) {
        self.todos = todos;
        if let Some(id) = &self.detail {
            if !self.todos.iter().any(|t| &t.id == id) {
                self.detail = None;
            }
        }
        self.clamp_selection();
    }

    pub fn set_memory_count(&mut self, count: usize) {
        self.memory_count = count;
    }

    pub fn set_channels(&mut self, channels: Vec<ChannelStatus>) {
        self.channels = channels;
        self.clamp_selection();
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = Some(status.into());
    }

    pub fn jobs(&self) -> &[SchedulerJob] {
        &self.jobs
    }

    pub fn todos(&self) -> &[TodoEntry] {
        &self.todos
    }

    pub fn memory_count(&self) -> usize {
        self.memory_count
    }

    pub fn channels(&self) -> &[ChannelStatus] {
        &self.channels
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    // ── Navigation ──

    pub fn section(&self) -> ControlSection {
        self.section
    }

    pub fn set_section(&mut self, section: ControlSection) {
        self.section = section;
        self.reset_cursor();
    }

    pub fn next_section(&mut self) {
        self.section = ControlSection::from_index(self.section.index() + 1);
        self.reset_cursor();
    }

    pub fn prev_section(&mut self) {
        self.section = ControlSection::from_index(self.section.index() + ControlSection::ALL.len() - 1);
        self.reset_cursor();
    }

    fn reset_cursor(&mut self) {
        self.selected = 0;
        self.scroll = 0;
        self.detail = None;
        self.editing = None;
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_rows
    }

    pub fn set_visible_rows(&mut self, rows: usize) {
        self.visible_rows = rows.max(1);
    }

    /// Number of selectable rows in the active section.
    pub fn item_count(&self) -> usize {
        match self.section {
            ControlSection::Scheduler => self.jobs.len(),
            ControlSection::Todos => self.todos.len(),
            ControlSection::Memories => 0,
            ControlSection::Channels => self.channels.len(),
        }
    }

    pub fn move_up(&mut self) {
        if self.editing.is_some() || self.selected == 0 {
            return;
        }
        self.selected -= 1;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn move_down(&mut self) {
        if self.editing.is_some() || self.item_count() == 0 {
            return;
        }
        if self.selected + 1 < self.item_count() {
            self.selected += 1;
            let visible = self.visible_rows;
            if self.selected >= self.scroll + visible {
                self.scroll = self.selected + 1 - visible;
            }
        }
    }

    fn clamp_selection(&mut self) {
        let count = self.item_count();
        if count == 0 {
            self.selected = 0;
            self.scroll = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
            self.scroll = self.scroll.min(self.selected);
        }
    }

    // ── Selection ──

    /// Handle Enter on the current row. Mutates local state and returns the
    /// side effect for `TuiApp` to perform.
    pub fn activate(&mut self) -> ControlAction {
        if self.editing.is_some() {
            return self.commit_edit();
        }

        match self.section {
            ControlSection::Scheduler => {
                let Some(job) = self.jobs.get_mut(self.selected) else {
                    return ControlAction::None;
                };
                job.enabled = !job.enabled;
                let action = ControlAction::ToggleScheduler {
                    schedule: job.schedule.clone(),
                    command: job.command.clone(),
                    enabled: job.enabled,
                };
                self.status = Some(format!(
                    "{} job: {} {}",
                    if job.enabled { "Enabled" } else { "Disabled" },
                    job.schedule,
                    job.command
                ));
                action
            }
            ControlSection::Todos => {
                let Some(todo) = self.todos.get(self.selected) else {
                    return ControlAction::None;
                };
                self.detail = Some(todo.id.clone());
                self.status = Some("Todo detail — press e to edit, Esc to go back".into());
                ControlAction::OpenTodo { id: todo.id.clone() }
            }
            ControlSection::Memories | ControlSection::Channels => ControlAction::None,
        }
    }

    // ── Todo editing ──

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn edit_buffer(&self) -> Option<&str> {
        self.editing.as_ref().map(|e| e.buffer.as_str())
    }

    pub fn begin_edit(&mut self) -> bool {
        if self.section != ControlSection::Todos {
            return false;
        }
        let Some(todo) = self.todos.get(self.selected) else {
            return false;
        };
        self.editing = Some(EditState {
            todo_id: todo.id.clone(),
            buffer: todo.title.clone(),
        });
        self.status = Some("Editing todo title — Enter to save, Esc to cancel".into());
        true
    }

    pub fn push_edit_char(&mut self, c: char) {
        if let Some(edit) = &mut self.editing {
            edit.buffer.push(c);
        }
    }

    pub fn pop_edit_char(&mut self) {
        if let Some(edit) = &mut self.editing {
            edit.buffer.pop();
        }
    }

    pub fn cancel_edit(&mut self) {
        self.editing = None;
        self.status = Some("Edit cancelled".into());
    }

    pub fn commit_edit(&mut self) -> ControlAction {
        let Some(edit) = self.editing.take() else {
            return ControlAction::None;
        };
        let title = edit.buffer.trim().to_string();
        if title.is_empty() {
            self.status = Some("Title cannot be empty — edit cancelled".into());
            return ControlAction::None;
        }
        let Some(todo) = self.todos.iter_mut().find(|t| t.id == edit.todo_id) else {
            return ControlAction::None;
        };
        todo.title = title.clone();
        self.status = Some(format!("Saved todo: {}", title));
        ControlAction::SaveTodo {
            id: edit.todo_id,
            title,
        }
    }

    // ── Detail ──

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub fn close_detail(&mut self) {
        self.detail = None;
        self.status = None;
    }

    pub fn detail_text(&self) -> Option<Vec<String>> {
        let id = self.detail.as_ref()?;
        let todo = self.todos.iter().find(|t| &t.id == id)?;
        Some(vec![
            format!("Todo  {}", todo.id),
            String::new(),
            format!("Title:    {}", todo.title),
            format!("Priority: {}", todo.priority),
            format!(
                "Status:   {}",
                if todo.completed { "completed" } else { "active" }
            ),
            String::new(),
            "Press e to edit the title, Esc to go back.".into(),
        ])
    }

    // ── Summary ──

    pub fn summary(&self) -> String {
        let on = self.jobs.iter().filter(|j| j.enabled).count();
        let active = self.todos.iter().filter(|t| !t.completed).count();
        let connected = self.channels.iter().filter(|c| c.connected).count();
        format!(
            "Scheduler: {}/{} on | Todos: {} active | Memories: {} | Channels: {}/{} connected",
            on,
            self.jobs.len(),
            active,
            self.memory_count,
            connected,
            self.channels.len()
        )
    }
}

// ── Rendering ───────────────────────────────────────────────────────

/// Render the control center overlay into `f`, using `app`'s theme.
pub fn render_control_center(app: &mut TuiApp, f: &mut Frame) {
    let area = f.area();
    let width = area.width.saturating_sub(4).clamp(40, 82);
    let height = area.height.saturating_sub(2).clamp(10, 26);
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let panel = Rect { x, y, width, height };

    let bg = Style::default().bg(app.theme.menu_bg);
    let text = Style::default().fg(app.theme.menu_text).bg(app.theme.menu_bg);
    let dim = Style::default().fg(app.theme.text_dim).bg(app.theme.menu_bg);

    f.render_widget(Block::default().style(bg), panel);

    let inner_x = x + 2;
    let inner_w = width.saturating_sub(4);

    // Title.
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Control Center",
            text.add_modifier(Modifier::BOLD),
        ))),
        Rect { x: inner_x, y, width: inner_w, height: 1 },
    );

    // Summary line.
    f.render_widget(
        Paragraph::new(app.control_center.summary()).style(dim),
        Rect { x: inner_x, y: y + 1, width: inner_w, height: 1 },
    );

    // Tab bar.
    let mut tabs = Vec::new();
    for (i, section) in ControlSection::ALL.iter().enumerate() {
        let active = *section == app.control_center.section();
        let label = if active {
            format!("[ {} ]", section.title().to_uppercase())
        } else {
            format!("  {}  ", section.title().to_uppercase())
        };
        let style = if active {
            Style::default()
                .fg(app.theme.menu_text)
                .bg(app.theme.menu_selected)
                .add_modifier(Modifier::BOLD)
        } else {
            dim
        };
        tabs.push(Span::styled(label, style));
        if i + 1 < ControlSection::ALL.len() {
            tabs.push(Span::styled(" ", bg));
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(tabs)),
        Rect { x: inner_x, y: y + 2, width: inner_w, height: 1 },
    );

    // Separator.
    f.render_widget(
        Paragraph::new("─".repeat(inner_w as usize)).style(dim),
        Rect { x: inner_x, y: y + 3, width: inner_w, height: 1 },
    );

    // Footer occupies the last two rows: status + key hints.
    let list_top = y + 4;
    let list_bottom = y + height - 3;
    let list_height = list_bottom.saturating_sub(list_top).max(1) as usize;
    app.control_center.set_visible_rows(list_height);

    let lines = body_lines(app, list_height);
    f.render_widget(
        Paragraph::new(Text::from(lines)).style(bg),
        Rect { x: inner_x, y: list_top, width: inner_w, height: list_height as u16 },
    );

    // Status line.
    let status = app
        .control_center
        .status()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "Ready".to_string());
    f.render_widget(
        Paragraph::new(status).style(dim),
        Rect { x: inner_x, y: y + height - 3, width: inner_w, height: 1 },
    );

    // Key hints.
    let keys = match app.control_center.section() {
        ControlSection::Scheduler => {
            "↑↓ Move  ←→ Section  Enter Toggle  r Refresh  Esc Close"
        }
        ControlSection::Todos => {
            "↑↓ Move  ←→ Section  Enter View  e Edit  r Refresh  Esc Close"
        }
        ControlSection::Memories | ControlSection::Channels => {
            "←→ Section  r Refresh  Esc Close"
        }
    };
    f.render_widget(
        Paragraph::new(keys).style(dim),
        Rect { x: inner_x, y: y + height - 2, width: inner_w, height: 1 },
    );
}

/// Build the rows shown in the list area, honoring edit/detail modes.
fn body_lines(app: &TuiApp, max_rows: usize) -> Vec<Line<'static>> {
    let cc = &app.control_center;
    let text = Style::default().fg(app.theme.menu_text).bg(app.theme.menu_bg);
    let selected_style = Style::default()
        .fg(app.theme.menu_text)
        .bg(app.theme.menu_selected)
        .add_modifier(Modifier::BOLD);

    if let Some(buffer) = cc.edit_buffer() {
        return vec![
            Line::from(Span::styled("Editing todo title", text)),
            Line::from(""),
            Line::from(Span::styled(format!("> {}_", buffer), selected_style)),
            Line::from(""),
            Line::from(Span::styled("Enter to save, Esc to cancel", text)),
        ];
    }

    if let Some(detail) = cc.detail_text() {
        return detail
            .into_iter()
            .map(|line| Line::from(Span::styled(line, text)))
            .collect();
    }

    match cc.section() {
        ControlSection::Scheduler => {
            if cc.jobs().is_empty() {
                return vec![Line::from(Span::styled("No scheduled jobs.", text))];
            }
            cc.jobs()
                .iter()
                .enumerate()
                .skip(cc.scroll())
                .take(max_rows)
                .map(|(i, job)| {
                    let style = if i == cc.selected() { selected_style } else { text };
                    let marker = if i == cc.selected() { ">" } else { " " };
                    let state = if job.enabled { "on " } else { "off" };
                    Line::from(Span::styled(
                        format!("{} [{state}] {} {}", marker, shift(job), job.command),
                        style,
                    ))
                })
                .collect()
        }
        ControlSection::Todos => {
            if cc.todos().is_empty() {
                return vec![Line::from(Span::styled("No active todos.", text))];
            }
            cc.todos()
                .iter()
                .enumerate()
                .skip(cc.scroll())
                .take(max_rows)
                .map(|(i, todo)| {
                    let style = if i == cc.selected() { selected_style } else { text };
                    let marker = if i == cc.selected() { ">" } else { " " };
                    let box_mark = if todo.completed { "x" } else { " " };
                    Line::from(Span::styled(
                        format!("{} [{}] {} ({})", marker, box_mark, todo.title, todo.priority),
                        style,
                    ))
                })
                .collect()
        }
        ControlSection::Memories => vec![
            Line::from(Span::styled(
                format!("Total memories: {}", cc.memory_count()),
                text,
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Memories are stored in the Obsidian vault.",
                text,
            )),
        ],
        ControlSection::Channels => {
            if cc.channels().is_empty() {
                return vec![Line::from(Span::styled("No channels configured.", text))];
            }
            cc.channels()
                .iter()
                .enumerate()
                .take(max_rows)
                .map(|(i, channel)| {
                    let style = if i == cc.selected() { selected_style } else { text };
                    let marker = if i == cc.selected() { ">" } else { " " };
                    let state = if channel.connected {
                        "connected"
                    } else {
                        "offline"
                    };
                    Line::from(Span::styled(
                        format!("{} {} [{}]", marker, channel.name, state),
                        style,
                    ))
                })
                .collect()
        }
    }
}

/// Left-pad the schedule column to 16 chars for alignment.
fn shift(job: &SchedulerJob) -> String {
    format!("{:<16}", job.schedule)
}

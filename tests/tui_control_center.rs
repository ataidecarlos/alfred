//! Integration tests for the TUI control center.
//!
//! Verification target for the "Add TUI control center for feature
//! management" work item: `cargo test --test tui_control_center`.
//!
//! These tests exercise the control-center state machine and the real
//! rendering path (`render_to_text`) without needing a running server or
//! touching the OS scheduler.

use alfred::scheduler::control::{mark_disabled, mark_enabled, merge_managed, CronEntry};
use alfred::tui::control::{
    ChannelStatus, ControlAction, ControlCenter, ControlSection, SchedulerJob, TodoEntry,
};
use alfred::tui::{render_to_text, TuiApp};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn sample_jobs() -> Vec<SchedulerJob> {
    vec![
        SchedulerJob::new("*/5 * * * *", "echo sync", true),
        SchedulerJob::new("0 9 * * *", "daily-summary", false),
    ]
}

fn sample_todos() -> Vec<TodoEntry> {
    vec![
        TodoEntry::new("t1", "Write docs", "high", false),
        TodoEntry::new("t2", "Fix bug", "medium", true),
    ]
}

fn sample_channels() -> Vec<ChannelStatus> {
    vec![
        ChannelStatus::new("api", true),
        ChannelStatus::new("telegram", false),
    ]
}

fn populated_center() -> ControlCenter {
    let mut cc = ControlCenter::new();
    cc.set_jobs(sample_jobs());
    cc.set_todos(sample_todos());
    cc.set_memory_count(7);
    cc.set_channels(sample_channels());
    cc
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

// ── State machine ───────────────────────────────────────────────────

#[test]
fn open_and_close() {
    let mut cc = ControlCenter::new();
    assert!(!cc.is_visible());

    cc.open();
    assert!(cc.is_visible());

    cc.close();
    assert!(!cc.is_visible());
}

#[test]
fn defaults_and_summary() {
    let cc = ControlCenter::new();
    assert_eq!(cc.section(), ControlSection::Scheduler);
    assert_eq!(cc.selected(), 0);
    assert_eq!(cc.item_count(), 0);

    let summary = cc.summary();
    assert!(summary.contains("Scheduler: 0/0 on"), "summary: {summary}");
    assert!(summary.contains("Memories: 0"), "summary: {summary}");
    assert!(summary.contains("Channels: 0/0 connected"), "summary: {summary}");
}

#[test]
fn arrow_keys_move_sections_and_clamp_items() {
    let mut cc = populated_center();
    assert_eq!(cc.section(), ControlSection::Scheduler);
    assert_eq!(cc.item_count(), 2);

    cc.move_down();
    assert_eq!(cc.selected(), 1);
    cc.move_down();
    assert_eq!(cc.selected(), 1, "selection clamps at the last job");
    cc.move_up();
    assert_eq!(cc.selected(), 0);
    cc.move_up();
    assert_eq!(cc.selected(), 0, "selection clamps at the first job");

    cc.next_section();
    assert_eq!(cc.section(), ControlSection::Todos);
    assert_eq!(cc.selected(), 0, "section change resets the cursor");
    cc.move_down();
    assert_eq!(cc.selected(), 1);

    cc.prev_section();
    assert_eq!(cc.section(), ControlSection::Scheduler);
    assert_eq!(cc.selected(), 0);
}

#[test]
fn sections_wrap_around() {
    let mut cc = ControlCenter::new();
    for expected in [
        ControlSection::Todos,
        ControlSection::Memories,
        ControlSection::Channels,
        ControlSection::Scheduler,
    ] {
        cc.next_section();
        assert_eq!(cc.section(), expected);
    }
    cc.prev_section();
    assert_eq!(cc.section(), ControlSection::Channels);
}

#[test]
fn enter_toggles_scheduler_job() {
    let mut cc = populated_center();
    assert_eq!(cc.section(), ControlSection::Scheduler);

    let action = cc.activate();
    assert_eq!(
        action,
        ControlAction::ToggleScheduler {
            schedule: "*/5 * * * *".into(),
            command: "echo sync".into(),
            enabled: false,
        }
    );
    assert!(!cc.jobs()[0].enabled, "first job toggled off locally");

    let action = cc.activate();
    assert_eq!(
        action,
        ControlAction::ToggleScheduler {
            schedule: "*/5 * * * *".into(),
            command: "echo sync".into(),
            enabled: true,
        }
    );
    assert!(cc.jobs()[0].enabled, "first job toggled back on");
}

#[test]
fn enter_on_todo_opens_detail() {
    let mut cc = populated_center();
    cc.set_section(ControlSection::Todos);

    let action = cc.activate();
    assert_eq!(action, ControlAction::OpenTodo { id: "t1".into() });
    assert_eq!(cc.detail(), Some("t1"));

    let detail = cc.detail_text().expect("detail for selected todo");
    assert!(detail.iter().any(|l| l.contains("Write docs")));
    assert!(detail.iter().any(|l| l.contains("high")));

    cc.close_detail();
    assert!(cc.detail().is_none());
}

#[test]
fn edit_todo_title_updates_local_state() {
    let mut cc = populated_center();
    cc.set_section(ControlSection::Todos);

    assert!(cc.begin_edit());
    assert!(cc.is_editing());
    assert_eq!(cc.edit_buffer(), Some("Write docs"));

    for _ in 0.."Write docs".len() {
        cc.pop_edit_char();
    }
    for c in "Ship release".chars() {
        cc.push_edit_char(c);
    }

    let action = cc.commit_edit();
    assert_eq!(
        action,
        ControlAction::SaveTodo {
            id: "t1".into(),
            title: "Ship release".into(),
        }
    );
    assert_eq!(cc.todos()[0].title, "Ship release");
    assert!(!cc.is_editing());
}

#[test]
fn empty_title_cancels_edit_without_change() {
    let mut cc = populated_center();
    cc.set_section(ControlSection::Todos);
    assert!(cc.begin_edit());

    for _ in 0..64 {
        cc.pop_edit_char();
    }
    let action = cc.commit_edit();
    assert_eq!(action, ControlAction::None);
    assert_eq!(cc.todos()[0].title, "Write docs", "title unchanged");
}

#[test]
fn edit_only_applies_to_todos_section() {
    let mut cc = populated_center();
    assert!(!cc.begin_edit(), "scheduler section has no editable rows");

    cc.set_section(ControlSection::Memories);
    assert!(!cc.begin_edit());
}

#[test]
fn shrinking_todos_clamps_selection_and_clears_detail() {
    let mut cc = populated_center();
    cc.set_section(ControlSection::Todos);
    cc.move_down();
    cc.activate(); // detail t2
    assert_eq!(cc.detail(), Some("t2"));

    cc.set_todos(vec![TodoEntry::new("t1", "Write docs", "high", false)]);
    assert_eq!(cc.item_count(), 1);
    assert_eq!(cc.selected(), 0, "selection clamped to new range");
    assert!(cc.detail().is_none(), "detail cleared when todo disappears");
}

// ── Rendering through the real TUI path ─────────────────────────────

#[test]
fn renders_all_sections_and_summary() {
    let mut app = TuiApp::new("http://localhost:1".into());
    {
        let cc = app.control_center_mut();
        cc.set_jobs(sample_jobs());
        cc.set_todos(sample_todos());
        cc.set_memory_count(7);
        cc.set_channels(sample_channels());
        cc.open();
    }

    let text = render_to_text(&mut app, 100, 40);
    assert!(text.contains("Control Center"), "title missing:\n{text}");
    assert!(text.contains("SCHEDULER"), "section tabs missing:\n{text}");
    assert!(text.contains("TODOS"));
    assert!(text.contains("MEMORIES"));
    assert!(text.contains("CHANNELS"));
    assert!(text.contains("*/5 * * * *"), "job schedule missing:\n{text}");
    assert!(text.contains("echo sync"), "job command missing:\n{text}");
    assert!(text.contains("Memories: 7"), "memory count missing:\n{text}");
    assert!(
        text.contains("Channels: 1/2 connected"),
        "channel summary missing:\n{text}"
    );
}

#[test]
fn renders_channels_section_and_status() {
    let mut app = TuiApp::new("http://localhost:1".into());
    {
        let cc = app.control_center_mut();
        cc.set_channels(sample_channels());
        cc.set_section(ControlSection::Channels);
        cc.open();
    }

    let text = render_to_text(&mut app, 100, 40);
    assert!(text.contains("[ CHANNELS ]"), "active tab missing:\n{text}");
    assert!(text.contains("api"), "api channel missing:\n{text}");
    assert!(text.contains("telegram"), "telegram channel missing:\n{text}");
    assert!(text.contains("connected"), "connected status missing:\n{text}");
    assert!(text.contains("offline"), "offline status missing:\n{text}");
}

#[test]
fn hidden_control_center_is_not_rendered() {
    let mut app = TuiApp::new("http://localhost:1".into());
    let text = render_to_text(&mut app, 100, 40);
    assert!(!text.contains("[ SCHEDULER ]"), "overlay rendered while closed");
}

#[test]
fn renders_todos_section_titles() {
    let mut app = TuiApp::new("http://localhost:1".into());
    {
        let cc = app.control_center_mut();
        cc.set_todos(sample_todos());
        cc.set_section(ControlSection::Todos);
        cc.open();
    }

    let text = render_to_text(&mut app, 100, 40);
    assert!(text.contains("[ TODOS ]"), "active tab marker missing:\n{text}");
    assert!(text.contains("Write docs"), "todo title missing:\n{text}");
    assert!(text.contains("Fix bug"), "second todo missing:\n{text}");
}

#[test]
fn renders_edit_mode_buffer() {
    let mut app = TuiApp::new("http://localhost:1".into());
    {
        let cc = app.control_center_mut();
        cc.set_todos(sample_todos());
        cc.set_section(ControlSection::Todos);
        cc.begin_edit();
        cc.open();
    }

    let text = render_to_text(&mut app, 100, 40);
    assert!(text.contains("Editing todo title"), "edit header missing:\n{text}");
    assert!(text.contains("Write docs_"), "edit buffer missing:\n{text}");
}

// ── Keyboard integration ────────────────────────────────────────────

#[test]
fn ctrl_k_opens_and_escape_closes_control_center() {
    let mut app = TuiApp::new("http://localhost:1".into());

    app.handle_key_event(ctrl('k'));
    assert!(app.control_center().is_visible());

    app.handle_key_event(key(KeyCode::Esc));
    assert!(!app.control_center().is_visible());
}

#[test]
fn keyboard_navigates_and_selects() {
    let mut app = TuiApp::new("http://localhost:1".into());
    {
        let cc = app.control_center_mut();
        cc.set_jobs(sample_jobs());
        cc.set_todos(sample_todos());
        cc.set_section(ControlSection::Todos);
    }

    app.handle_key_event(ctrl('k')); // open
    assert!(app.control_center().is_visible());
    assert_eq!(app.control_center().section(), ControlSection::Todos);

    app.handle_key_event(key(KeyCode::Down));
    assert_eq!(app.control_center().selected(), 1);

    app.handle_key_event(key(KeyCode::Enter));
    assert_eq!(app.control_center().detail(), Some("t2"));

    // Escape leaves the detail open, a second Escape closes the overlay.
    app.handle_key_event(key(KeyCode::Esc));
    assert!(app.control_center().is_visible());
    assert!(app.control_center().detail().is_none());
    app.handle_key_event(key(KeyCode::Esc));
    assert!(!app.control_center().is_visible());
}

#[test]
fn keyboard_toggles_scheduler_job() {
    let mut app = TuiApp::new("http://localhost:1".into());
    app.control_center_mut().set_jobs(sample_jobs());

    app.handle_key_event(ctrl('k')); // open on Scheduler section
    app.handle_key_event(key(KeyCode::Enter));

    assert!(!app.control_center().jobs()[0].enabled);
    assert!(app.control_center().status().unwrap().contains("Disabled"));
}

// ── Scheduler backend helpers (shared with the toggle action) ───────

#[test]
fn merge_managed_marks_active_and_disabled_jobs() {
    let active = vec![CronEntry::new("*/5 * * * *", "echo sync")];
    let disabled = vec![
        CronEntry::new("0 9 * * *", "daily-summary"),
        // Duplicate of an active job must not be listed twice.
        CronEntry::new("*/5 * * * *", "echo sync"),
    ];

    let merged = merge_managed(active, disabled);
    assert_eq!(merged.len(), 2);
    assert!(merged[0].enabled);
    assert_eq!(merged[0].schedule, "*/5 * * * *");
    assert!(!merged[1].enabled);
    assert_eq!(merged[1].command, "daily-summary");
}

#[test]
fn disabled_manifest_helpers_are_idempotent() {
    let entry = CronEntry::new("0 9 * * *", "daily-summary");

    let once = mark_disabled(&[], &entry);
    assert_eq!(once.len(), 1);
    let twice = mark_disabled(&once, &entry);
    assert_eq!(twice.len(), 1, "disabling twice does not duplicate");

    let enabled = mark_enabled(&twice, &entry);
    assert!(enabled.is_empty(), "enabling removes the entry");
}

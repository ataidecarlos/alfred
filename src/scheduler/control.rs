//! OS-agnostic scheduled task control.
//!
//! Exposes a small unified interface for the CLI (`alfred scheduler ...`) that
//! wraps the host operating system's native scheduler:
//!
//! * Unix (Linux/macOS): the user crontab via `crontab`
//! * Windows: Task Scheduler via `schtasks`, indexed by a local manifest
//!
//! The pure helpers ([`parse_entry`], [`upsert`], [`remove_from`],
//! [`validate_schedule`]) are unit tested; the thin platform modules only
//! shell out to the OS tools.

use std::io::Write;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::error::AlfredError;

/// A single scheduled entry: a cron expression plus the command to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronEntry {
    pub schedule: String,
    pub command: String,
}

impl CronEntry {
    pub fn new(schedule: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            schedule: schedule.into(),
            command: command.into(),
        }
    }

    /// Render the entry as a crontab line.
    pub fn to_line(&self) -> String {
        format!("{} {}", self.schedule, self.command)
    }

    /// Canonicalize whitespace by re-parsing the rendered line, so entries
    /// built from raw user input compare equal to parsed crontab lines.
    pub fn normalized(&self) -> CronEntry {
        parse_entry(&self.to_line()).unwrap_or_else(|| self.clone())
    }
}

/// Schedule keywords accepted in addition to a 5-field cron expression.
const SPECIAL_SCHEDULES: [&str; 8] = [
    "@reboot", "@yearly", "@annually", "@monthly", "@weekly", "@daily", "@midnight", "@hourly",
];

/// Parse a single crontab line into a [`CronEntry`].
///
/// Returns `None` for blank lines, comments, and environment assignments
/// (which do not look like cron entries).
pub fn parse_entry(line: &str) -> Option<CronEntry> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();

    // @reboot / @daily style entries carry no schedule fields.
    if tokens.len() >= 2 && tokens[0].starts_with('@') {
        return Some(CronEntry::new(tokens[0], tokens[1..].join(" ")));
    }

    // Standard entries need 5 schedule fields plus a command.
    if tokens.len() < 6 {
        return None;
    }

    Some(CronEntry::new(tokens[..5].join(" "), tokens[5..].join(" ")))
}

/// Validate a cron schedule before it is written to the OS scheduler.
pub fn validate_schedule(schedule: &str) -> Result<(), AlfredError> {
    let schedule = schedule.trim();
    if schedule.is_empty() || schedule.contains('\n') || schedule.contains('\r') {
        return Err(AlfredError::Scheduler("schedule must not be empty".into()));
    }

    if schedule.starts_with('@') {
        if SPECIAL_SCHEDULES.contains(&schedule) {
            return Ok(());
        }
        return Err(AlfredError::Scheduler(format!(
            "unsupported schedule keyword `{}`",
            schedule
        )));
    }

    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(AlfredError::Scheduler(format!(
            "cron schedule must have 5 fields, got {}: `{}`",
            fields.len(),
            schedule
        )));
    }

    Ok(())
}

/// Validate a command before it is written to the OS scheduler.
pub fn validate_command(command: &str) -> Result<(), AlfredError> {
    if command.trim().is_empty() {
        return Err(AlfredError::Scheduler("command must not be empty".into()));
    }
    if command.contains('\n') || command.contains('\r') {
        return Err(AlfredError::Scheduler(
            "command must not contain newlines".into(),
        ));
    }
    Ok(())
}

/// Insert `entry` into `lines`, avoiding exact duplicates.
pub fn upsert(lines: &[String], entry: &CronEntry) -> Vec<String> {
    let entry = entry.normalized();
    let mut result: Vec<String> = lines.to_vec();
    let already_present = result
        .iter()
        .any(|line| parse_entry(line).as_ref() == Some(&entry));
    if !already_present {
        result.push(entry.to_line());
    }
    result
}

/// Remove every line matching `entry`. Returns the new lines and whether an
/// entry was actually removed.
pub fn remove_from(lines: &[String], entry: &CronEntry) -> (Vec<String>, bool) {
    let entry = entry.normalized();
    let mut removed = false;
    let remaining = lines
        .iter()
        .filter(|line| {
            if parse_entry(line).as_ref() == Some(&entry) {
                removed = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    (remaining, removed)
}

/// Render a list of entries for terminal output.
pub fn format_jobs(entries: &[CronEntry]) -> String {
    if entries.is_empty() {
        return "No scheduled jobs.".to_string();
    }

    let mut out = String::new();
    out.push_str(&format!("{:<20} {}\n", "SCHEDULE", "COMMAND"));
    out.push_str(&"-".repeat(60));
    out.push('\n');
    for entry in entries {
        out.push_str(&format!("{:<20} {}\n", entry.schedule, entry.command));
    }
    out
}

/// List the scheduled jobs managed by the OS scheduler.
pub fn list_jobs() -> Result<Vec<CronEntry>, AlfredError> {
    if cfg!(windows) {
        windows::list()
    } else {
        let lines = unix::read_lines()?;
        Ok(lines.iter().filter_map(|line| parse_entry(line)).collect())
    }
}

/// Add a scheduled job to the OS scheduler.
pub fn add_job(schedule: &str, command: &str) -> Result<CronEntry, AlfredError> {
    validate_schedule(schedule)?;
    validate_command(command)?;
    let entry = CronEntry::new(schedule.trim(), command.trim());

    if cfg!(windows) {
        windows::add(&entry)?;
    } else {
        let lines = unix::read_lines()?;
        let updated = upsert(&lines, &entry);
        unix::write_lines(&updated)?;
    }

    Ok(entry)
}

/// Remove a scheduled job from the OS scheduler. Returns whether it existed.
pub fn remove_job(schedule: &str, command: &str) -> Result<bool, AlfredError> {
    validate_schedule(schedule)?;
    validate_command(command)?;
    let entry = CronEntry::new(schedule.trim(), command.trim());

    if cfg!(windows) {
        windows::remove(&entry)
    } else {
        let lines = unix::read_lines()?;
        let (updated, removed) = remove_from(&lines, &entry);
        if removed {
            unix::write_lines(&updated)?;
        }
        Ok(removed)
    }
}

/// A scheduled job plus whether it is currently enabled.
///
/// Disabled jobs are removed from the OS scheduler and remembered in a local
/// manifest so the control center can list and re-enable them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedJob {
    pub schedule: String,
    pub command: String,
    pub enabled: bool,
}

impl ManagedJob {
    pub fn new(entry: CronEntry, enabled: bool) -> Self {
        Self {
            schedule: entry.schedule,
            command: entry.command,
            enabled,
        }
    }

    pub fn entry(&self) -> CronEntry {
        CronEntry::new(&self.schedule, &self.command)
    }
}

/// Path of the manifest recording jobs that were disabled (toggled off).
fn disabled_manifest_path() -> std::path::PathBuf {
    crate::paths::Paths::data_dir().join("disabled_jobs.json")
}

fn load_disabled() -> Vec<CronEntry> {
    match std::fs::read_to_string(disabled_manifest_path()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_disabled(entries: &[CronEntry]) -> Result<(), AlfredError> {
    let path = disabled_manifest_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(entries)
        .map_err(|e| AlfredError::Scheduler(e.to_string()))?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Merge enabled (active) and disabled entries into one list. Active jobs keep
/// their OS order; disabled jobs are appended, skipping duplicates.
pub fn merge_managed(active: Vec<CronEntry>, disabled: Vec<CronEntry>) -> Vec<ManagedJob> {
    let mut jobs: Vec<ManagedJob> = active
        .into_iter()
        .map(|entry| ManagedJob::new(entry, true))
        .collect();
    for entry in disabled {
        let normalized = entry.normalized();
        if !jobs.iter().any(|job| job.entry().normalized() == normalized) {
            jobs.push(ManagedJob::new(entry, false));
        }
    }
    jobs
}

/// Add `entry` to the disabled manifest, avoiding duplicates.
pub fn mark_disabled(disabled: &[CronEntry], entry: &CronEntry) -> Vec<CronEntry> {
    let normalized = entry.normalized();
    let mut result: Vec<CronEntry> = disabled
        .iter()
        .filter(|existing| existing.normalized() != normalized)
        .cloned()
        .collect();
    result.push(entry.clone());
    result
}

/// Remove `entry` from the disabled manifest.
pub fn mark_enabled(disabled: &[CronEntry], entry: &CronEntry) -> Vec<CronEntry> {
    let normalized = entry.normalized();
    disabled
        .iter()
        .filter(|existing| existing.normalized() != normalized)
        .cloned()
        .collect()
}

/// List every managed job: enabled ones from the OS scheduler plus disabled
/// ones remembered in the local manifest.
pub fn list_managed_jobs() -> Result<Vec<ManagedJob>, AlfredError> {
    let active = list_jobs()?;
    let disabled = load_disabled();
    Ok(merge_managed(active, disabled))
}

/// Enable or disable a job. Disabling removes it from the OS scheduler and
/// records it in the manifest; enabling does the reverse.
pub fn set_job_enabled(job: &ManagedJob, enabled: bool) -> Result<(), AlfredError> {
    let entry = job.entry();
    if enabled {
        add_job(&entry.schedule, &entry.command)?;
        let disabled = load_disabled();
        save_disabled(&mark_enabled(&disabled, &entry))?;
    } else {
        remove_job(&entry.schedule, &entry.command)?;
        let disabled = load_disabled();
        save_disabled(&mark_disabled(&disabled, &entry))?;
    }
    Ok(())
}

/// Unix backend backed by the user crontab.
mod unix {
    use super::*;

    pub fn read_lines() -> Result<Vec<String>, AlfredError> {
        let output = match Command::new("crontab").arg("-l").output() {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AlfredError::Scheduler(
                    "`crontab` was not found; OS cron is unavailable on this system".into(),
                ));
            }
            Err(e) => return Err(AlfredError::Io(e)),
        };

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| line.to_string())
                .collect())
        } else {
            // `crontab -l` exits non-zero when the user has no crontab yet.
            Ok(Vec::new())
        }
    }

    pub fn write_lines(lines: &[String]) -> Result<(), AlfredError> {
        let mut content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }

        let mut child = match Command::new("crontab")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AlfredError::Scheduler(
                    "`crontab` was not found; OS cron is unavailable on this system".into(),
                ));
            }
            Err(e) => return Err(AlfredError::Io(e)),
        };

        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(content.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(AlfredError::Scheduler(format!(
                "failed to write crontab: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }

        Ok(())
    }
}

/// Windows backend backed by Task Scheduler (`schtasks`).
///
/// Task names are derived deterministically from the entry so a job can be
/// removed without keeping external state. A small manifest is kept so `list`
/// can render schedules and commands rather than opaque task names.
mod windows {
    use super::*;

    fn manifest_path() -> std::path::PathBuf {
        crate::paths::Paths::data_dir().join("scheduled_tasks.json")
    }

    fn load_manifest() -> Vec<CronEntry> {
        match std::fs::read_to_string(manifest_path()) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    fn save_manifest(entries: &[CronEntry]) -> Result<(), AlfredError> {
        let path = manifest_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content =
            serde_json::to_string_pretty(entries).map_err(|e| AlfredError::Scheduler(e.to_string()))?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Stable FNV-1a hash so the same entry always maps to the same task name.
    fn task_name(entry: &CronEntry) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in entry.to_line().bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("AlfredScheduled_{:016x}", hash)
    }

    fn create_args(entry: &CronEntry) -> Result<Vec<String>, AlfredError> {
        let mut args: Vec<String> = vec![
            "/create".into(),
            "/tn".into(),
            task_name(entry),
            "/tr".into(),
            entry.command.clone(),
            "/sc".into(),
        ];

        let schedule = entry.schedule.trim();
        if schedule.starts_with('@') {
            let sc = match schedule {
                "@hourly" => "HOURLY",
                "@daily" | "@midnight" => "DAILY",
                "@weekly" => "WEEKLY",
                "@monthly" => "MONTHLY",
                _ => {
                    return Err(AlfredError::Scheduler(format!(
                        "schedule `{}` is not supported on Windows",
                        schedule
                    )));
                }
            };
            args.push(sc.into());
        } else {
            let parts: Vec<&str> = schedule.split_whitespace().collect();
            if parts.len() != 5 {
                return Err(AlfredError::Scheduler(format!(
                    "invalid cron schedule: `{}`",
                    schedule
                )));
            }
            let (minute, hour, dom, month, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4]);

            if let Some(step) = minute.strip_prefix("*/") {
                if hour != "*" || dom != "*" || month != "*" || dow != "*" {
                    return Err(AlfredError::Scheduler(
                        "only `*/N * * * *` minute intervals are supported on Windows".into(),
                    ));
                }
                let step: u32 = step
                    .parse()
                    .map_err(|_| AlfredError::Scheduler("invalid minute interval".into()))?;
                if step == 0 || step > 59 {
                    return Err(AlfredError::Scheduler(
                        "minute interval must be between 1 and 59".into(),
                    ));
                }
                args.push("MINUTE".into());
                args.push("/mo".into());
                args.push(step.to_string());
            } else if dom == "*" && month == "*" && dow == "*" {
                let minute: u32 = minute
                    .parse()
                    .map_err(|_| AlfredError::Scheduler("invalid minute field".into()))?;
                let hour: u32 = hour
                    .parse()
                    .map_err(|_| AlfredError::Scheduler("invalid hour field".into()))?;
                if minute > 59 || hour > 23 {
                    return Err(AlfredError::Scheduler(
                        "invalid time of day for daily schedule".into(),
                    ));
                }
                args.push("DAILY".into());
                args.push("/st".into());
                args.push(format!("{:02}:{:02}", hour, minute));
            } else {
                return Err(AlfredError::Scheduler(
                    "this cron pattern is not supported on Windows; use `*/N * * * *`, `M H * * *`, or an @keyword".into(),
                ));
            }
        }

        args.push("/f".into());
        Ok(args)
    }

    fn run_schtasks(args: &[String]) -> Result<(), AlfredError> {
        let output = match Command::new("schtasks").args(args).output() {
            Ok(output) => output,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AlfredError::Scheduler(
                    "`schtasks` was not found; Task Scheduler is unavailable on this system".into(),
                ));
            }
            Err(e) => return Err(AlfredError::Io(e)),
        };

        if !output.status.success() {
            return Err(AlfredError::Scheduler(format!(
                "schtasks failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    pub fn list() -> Result<Vec<CronEntry>, AlfredError> {
        Ok(load_manifest())
    }

    pub fn add(entry: &CronEntry) -> Result<(), AlfredError> {
        let args = create_args(entry)?;
        run_schtasks(&args)?;

        let mut entries = load_manifest();
        if !entries.contains(entry) {
            entries.push(entry.clone());
            save_manifest(&entries)?;
        }
        Ok(())
    }

    pub fn remove(entry: &CronEntry) -> Result<bool, AlfredError> {
        let mut entries = load_manifest();
        if !entries.contains(entry) {
            return Ok(false);
        }

        let args = vec![
            "/delete".to_string(),
            "/tn".to_string(),
            task_name(entry),
            "/f".to_string(),
        ];
        run_schtasks(&args)?;

        entries.retain(|existing| existing != entry);
        save_manifest(&entries)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_entry() {
        let entry = parse_entry("*/5 * * * * echo test").unwrap();
        assert_eq!(entry.schedule, "*/5 * * * *");
        assert_eq!(entry.command, "echo test");
    }

    #[test]
    fn parses_special_entry() {
        let entry = parse_entry("@reboot /usr/bin/alfred").unwrap();
        assert_eq!(entry.schedule, "@reboot");
        assert_eq!(entry.command, "/usr/bin/alfred");
    }

    #[test]
    fn ignores_comments_blanks_and_env() {
        assert!(parse_entry("# a comment").is_none());
        assert!(parse_entry("").is_none());
        assert!(parse_entry("   ").is_none());
        assert!(parse_entry("PATH=/usr/bin").is_none());
    }

    #[test]
    fn validates_schedules() {
        assert!(validate_schedule("*/5 * * * *").is_ok());
        assert!(validate_schedule("@daily").is_ok());
        assert!(validate_schedule("*/5 * * *").is_err());
        assert!(validate_schedule("@nonsense").is_err());
        assert!(validate_schedule("").is_err());
        assert!(validate_schedule("*/5 * * * *\nrm -rf /").is_err());
    }

    #[test]
    fn rejects_newline_in_command() {
        assert!(validate_command("echo hi").is_ok());
        assert!(validate_command("echo hi\nrm -rf /").is_err());
        assert!(validate_command("   ").is_err());
    }

    #[test]
    fn upsert_does_not_duplicate() {
        let entry = CronEntry::new("*/5 * * * *", "echo test");
        let lines = vec!["0 9 * * * backup".to_string()];
        let once = upsert(&lines, &entry);
        assert_eq!(once.len(), 2);
        assert_eq!(once[1], "*/5 * * * * echo test");

        let twice = upsert(&once, &entry);
        assert_eq!(twice.len(), 2);
    }

    #[test]
    fn remove_from_reports_removal() {
        let entry = CronEntry::new("*/5 * * * *", "echo test");
        let lines = vec![
            "0 9 * * * backup".to_string(),
            "*/5 * * * * echo test".to_string(),
        ];
        let (remaining, removed) = remove_from(&lines, &entry);
        assert!(removed);
        assert_eq!(remaining, vec!["0 9 * * * backup".to_string()]);

        let (_, removed_again) = remove_from(&remaining, &entry);
        assert!(!removed_again);
    }

    #[test]
    fn removes_entry_with_normalized_whitespace() {
        let entry = CronEntry::new("*/5 * * * *", "echo   test");
        let lines = vec!["*/5 * * * * echo   test".to_string()];
        let (remaining, removed) = remove_from(&lines, &entry);
        assert!(removed);
        assert!(remaining.is_empty());
    }

    #[test]
    fn formats_empty_and_populated_jobs() {
        assert_eq!(format_jobs(&[]), "No scheduled jobs.");
        let jobs = vec![CronEntry::new("*/5 * * * *", "echo test")];
        let rendered = format_jobs(&jobs);
        assert!(rendered.contains("SCHEDULE"));
        assert!(rendered.contains("echo test"));
    }
}

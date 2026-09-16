use serde::{Deserialize, Serialize};

use crate::error::AlfredError;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryFrontmatter {
    pub title: String,
    #[serde(rename = "type")]
    pub mem_type: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default = "default_status")]
    pub status: String,
    pub created: String,
    pub updated: String,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub retrieval_count: i64,
    #[serde(default)]
    pub last_retrieved: Option<String>,
    #[serde(default = "default_importance")]
    pub importance: String,
    #[serde(default)]
    pub distilled_from: Option<String>,
}

fn default_status() -> String { "active".into() }
fn default_source() -> String { "conversation".into() }
fn default_importance() -> String { "medium".into() }

pub fn parse(content: &str) -> Result<(MemoryFrontmatter, String), AlfredError> {
    let content = content.trim();

    if !content.starts_with("---") {
        return Err(AlfredError::Config("No frontmatter found".into()));
    }

    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return Err(AlfredError::Config("Malformed frontmatter".into()));
    }

    let yaml_str = parts[1].trim();
    let body = parts[2].trim().to_string();

    let fm: MemoryFrontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| AlfredError::Config(format!("YAML parse error: {}", e)))?;

    Ok((fm, body))
}

pub fn serialize(fm: &MemoryFrontmatter, body: &str) -> String {
    let yaml = serde_yaml::to_string(fm).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, body)
}

pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]").unwrap();
    re.captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

pub fn add_wikilink(content: &str, target: &str) -> String {
    let link = format!("[[{}]]", target);
    if content.contains(&link) {
        return content.to_string();
    }

    let mut result = content.to_string();
    if !result.contains("## Related") {
        result.push_str("\n\n## Related\n\n");
    }
    result.push_str(&format!("- {}\n", link));
    result
}

pub fn remove_wikilink(content: &str, target: &str) -> String {
    let link = format!("[[{}]]", target);
    let line = format!("- {}\n", link);
    content.replace(&line, "")
}

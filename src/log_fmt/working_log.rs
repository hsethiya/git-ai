use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Line {
    Single(u32),
    Range(u32, u32),
}

#[allow(dead_code)]
impl Line {
    pub fn start(&self) -> u32 {
        match self {
            Line::Single(line) => *line,
            Line::Range(start, _) => *start,
        }
    }

    /// Get the end line #C
    /// CLAUDE SAYS HI
    pub fn end(&self) -> u32 {
        match self {
            Line::Single(line) => *line,
            Line::Range(_, end) => *end,
        }
    }

    /// Check if this line/range contains a given line number
    pub fn contains(&self, line_number: u32) -> bool {
        match self {
            Line::Single(line) => *line == line_number,
            Line::Range(start, end) => line_number >= *start && line_number <= *end,
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Line::Single(line) => write!(f, "{}", line),
            Line::Range(start, end) => write!(f, "[{}, {}]", start, end),
        }
    }
}

/// Represents a working log entry for a specific file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingLogEntry {
    /// The file path relative to the repository root
    pub file: String,
    /// List of lines or line ranges that were added
    pub added_lines: Vec<Line>,
    /// List of lines or line ranges that were deleted
    pub deleted_lines: Vec<Line>,
}

#[allow(dead_code)]
impl WorkingLogEntry {
    /// Create a new working log entry
    pub fn new(file: String, added_lines: Vec<Line>, deleted_lines: Vec<Line>) -> Self {
        Self {
            file,
            added_lines,
            deleted_lines,
        }
    }

    /// Add a single line to the added lines
    pub fn add_added_line(&mut self, line: u32) {
        self.added_lines.push(Line::Single(line));
    }

    /// Add a line range to the added lines
    pub fn add_added_range(&mut self, start: u32, end: u32) {
        self.added_lines.push(Line::Range(start, end));
    }

    /// Add a single line to the deleted lines
    pub fn add_deleted_line(&mut self, line: u32) {
        self.deleted_lines.push(Line::Single(line));
    }

    /// Add a line range to the deleted lines
    pub fn add_deleted_range(&mut self, start: u32, end: u32) {
        self.deleted_lines.push(Line::Range(start, end));
    }

    /// Check if a specific line number is covered by this working log entry
    pub fn covers_line(&self, line_number: u32) -> bool {
        self.added_lines
            .iter()
            .any(|line| line.contains(line_number))
            || self
                .deleted_lines
                .iter()
                .any(|line| line.contains(line_number))
    }

    /// Get all lines (both added and deleted) for backward compatibility
    pub fn all_lines(&self) -> Vec<Line> {
        let mut all_lines = Vec::new();
        all_lines.extend(self.added_lines.clone());
        all_lines.extend(self.deleted_lines.clone());
        all_lines
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub model: String,
    pub human_author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub text: String,
    pub role: PromptRole,
    pub username: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptRole {
    User,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentId {
    pub tool: String, // e.g., "cursor", "windsurf"
    pub id: String,   // id in their domain
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub messages: Vec<PromptMessage>,
    pub agent_id: AgentId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub snapshot: String,
    pub diff: String,
    pub author: String,
    pub entries: Vec<WorkingLogEntry>,
    pub timestamp: u64,
    pub agent_metadata: Option<AgentMetadata>,
    pub prompt: Option<Prompt>,
}

impl Checkpoint {
    pub fn new(
        snapshot: String,
        diff: String,
        author: String,
        entries: Vec<WorkingLogEntry>,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            snapshot,
            diff,
            author,
            entries,
            timestamp,
            agent_metadata: None,
            prompt: None,
        }
    }

    pub fn new_with_metadata(
        snapshot: String,
        diff: String,
        author: String,
        entries: Vec<WorkingLogEntry>,
        agent_metadata: AgentMetadata,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            snapshot,
            diff,
            author,
            entries,
            timestamp,
            agent_metadata: Some(agent_metadata),
            prompt: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_serialization() {
        let single_line = Line::Single(5);
        let range_line = Line::Range(10, 15);

        let single_json = serde_json::to_string(&single_line).unwrap();
        let range_json = serde_json::to_string(&range_line).unwrap();

        assert_eq!(single_json, "5");
        assert_eq!(range_json, "[10,15]");

        let deserialized_single: Line = serde_json::from_str(&single_json).unwrap();
        let deserialized_range: Line = serde_json::from_str(&range_json).unwrap();

        assert_eq!(deserialized_single, single_line);
        assert_eq!(deserialized_range, range_line);
    }

    #[test]
    fn test_checkpoint_serialization() {
        let entry = WorkingLogEntry::new(
            "src/xyz.rs".to_string(),
            vec![Line::Single(1), Line::Range(2, 5), Line::Single(10)],
            vec![],
        );
        let checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry],
        );

        // Verify timestamp is set (should be recent)
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        assert!(checkpoint.timestamp > 0);
        assert!(checkpoint.timestamp <= current_time);
        assert!(checkpoint.agent_metadata.is_none());
        assert!(checkpoint.prompt.is_none());

        let json = serde_json::to_string_pretty(&checkpoint).unwrap();
        let deserialized: Checkpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.snapshot, "abc123");
        assert_eq!(deserialized.diff, "");
        assert_eq!(deserialized.entries.len(), 1);
        assert_eq!(deserialized.entries[0].file, "src/xyz.rs");
        assert_eq!(deserialized.timestamp, checkpoint.timestamp);
        assert!(deserialized.agent_metadata.is_none());
        assert!(deserialized.prompt.is_none());
    }

    #[test]
    fn test_log_array_serialization() {
        let entry1 = WorkingLogEntry::new(
            "src/xyz.rs".to_string(),
            vec![Line::Single(1), Line::Range(2, 5), Line::Single(10)],
            vec![],
        );
        let checkpoint1 = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry1],
        );

        let entry2 = WorkingLogEntry::new(
            "src/xyz.rs".to_string(),
            vec![Line::Single(12), Line::Single(13)],
            vec![],
        );
        let checkpoint2 = Checkpoint::new(
            "def456".to_string(),
            "/refs/ai/working/xyz.patch".to_string(),
            "user".to_string(),
            vec![entry2],
        );

        // Verify timestamps are set and checkpoint2 is newer than checkpoint1
        assert!(checkpoint1.timestamp > 0);
        assert!(checkpoint2.timestamp > 0);
        assert!(checkpoint2.timestamp >= checkpoint1.timestamp);

        let log = vec![checkpoint1, checkpoint2];
        let json = serde_json::to_string_pretty(&log).unwrap();
        // println!("Working log array JSON:\n{}", json);
        let deserialized: Vec<Checkpoint> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized[0].snapshot, "abc123");
        assert_eq!(deserialized[1].snapshot, "def456");
        assert_eq!(deserialized[1].author, "user");
    }

    #[test]
    fn test_line_contains() {
        let single = Line::Single(5);
        let range = Line::Range(10, 15);

        assert!(single.contains(5));
        assert!(!single.contains(6));

        assert!(range.contains(10));
        assert!(range.contains(12));
        assert!(range.contains(15));
        assert!(!range.contains(9));
        assert!(!range.contains(16));
    }

    #[test]
    fn test_working_log_entry_covers_line() {
        let entry = WorkingLogEntry::new(
            "src/xyz.rs".to_string(),
            vec![Line::Single(1), Line::Range(2, 5), Line::Single(10)],
            vec![],
        );

        assert!(entry.covers_line(1));
        assert!(entry.covers_line(2));
        assert!(entry.covers_line(5));
        assert!(entry.covers_line(10));
        assert!(!entry.covers_line(6));
        assert!(!entry.covers_line(11));
    }

    #[test]
    fn test_checkpoint_with_agent_metadata() {
        let entry = WorkingLogEntry::new("src/xyz.rs".to_string(), vec![Line::Single(1)], vec![]);

        let agent_metadata = AgentMetadata {
            model: "claude-3-sonnet".to_string(),
            human_author: Some("john.doe".to_string()),
        };

        let checkpoint = Checkpoint::new_with_metadata(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry],
            agent_metadata,
        );

        assert!(checkpoint.agent_metadata.is_some());
        let metadata = checkpoint.agent_metadata.as_ref().unwrap();
        assert_eq!(metadata.model, "claude-3-sonnet");
        assert_eq!(metadata.human_author.as_deref(), Some("john.doe"));

        let json = serde_json::to_string_pretty(&checkpoint).unwrap();
        let deserialized: Checkpoint = serde_json::from_str(&json).unwrap();
        assert!(deserialized.agent_metadata.is_some());
        let deserialized_metadata = deserialized.agent_metadata.as_ref().unwrap();
        assert_eq!(deserialized_metadata.model, "claude-3-sonnet");
        assert_eq!(
            deserialized_metadata.human_author.as_deref(),
            Some("john.doe")
        );
    }

    #[test]
    fn test_checkpoint_with_prompt() {
        let entry = WorkingLogEntry::new("src/xyz.rs".to_string(), vec![Line::Single(1)], vec![]);

        let prompt_message = PromptMessage {
            text: "Please add error handling to this function".to_string(),
            role: PromptRole::User,
            username: Some("john.doe".to_string()),
            timestamp: 1234567890,
        };

        let agent_id = AgentId {
            tool: "cursor".to_string(),
            id: "session-abc123".to_string(),
        };

        let prompt = Prompt {
            messages: vec![prompt_message],
            agent_id,
        };

        let mut checkpoint = Checkpoint::new(
            "abc123".to_string(),
            "".to_string(),
            "claude".to_string(),
            vec![entry],
        );
        checkpoint.prompt = Some(prompt);

        assert!(checkpoint.prompt.is_some());
        let prompt_data = checkpoint.prompt.as_ref().unwrap();
        assert_eq!(prompt_data.messages.len(), 1);
        assert_eq!(
            prompt_data.messages[0].text,
            "Please add error handling to this function"
        );
        assert!(matches!(prompt_data.messages[0].role, PromptRole::User));
        assert_eq!(
            prompt_data.messages[0].username.as_deref(),
            Some("john.doe")
        );
        assert_eq!(prompt_data.messages[0].timestamp, 1234567890);
        assert_eq!(prompt_data.agent_id.tool, "cursor");
        assert_eq!(prompt_data.agent_id.id, "session-abc123");

        let json = serde_json::to_string_pretty(&checkpoint).unwrap();
        let deserialized: Checkpoint = serde_json::from_str(&json).unwrap();
        assert!(deserialized.prompt.is_some());
        let deserialized_prompt = deserialized.prompt.as_ref().unwrap();
        assert_eq!(deserialized_prompt.messages.len(), 1);
        assert_eq!(deserialized_prompt.agent_id.tool, "cursor");
        assert_eq!(deserialized_prompt.agent_id.id, "session-abc123");
    }
}

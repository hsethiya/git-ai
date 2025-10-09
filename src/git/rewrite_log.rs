use crate::error::GitAiError;
use serde::{Deserialize, Serialize};

/// Simple case classes for rewrite events
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RewriteLogEvent {
    Merge {
        merge: MergeEvent,
    },
    MergeSquash {
        merge_squash: MergeSquashEvent,
    },
    RebaseInteractive {
        rebase_interactive: RebaseInteractiveEvent,
    },
    Rebase {
        rebase: RebaseEvent,
    },
    CherryPick {
        cherry_pick: CherryPickEvent,
    },
    RevertMixed {
        revert_mixed: RevertMixedEvent,
    },
    ResetSoft {
        reset_soft: ResetSoftEvent,
    },
    ResetHard {
        reset_hard: ResetHardEvent,
    },
    CommitAmend {
        commit_amend: CommitAmendEvent,
    },
    Commit {
        commit: CommitEvent,
    },
    Stash {
        stash: StashEvent,
    },
    AuthorshipLogsSynced {
        authorship_logs_synced: AuthorshipLogsSyncedEvent,
    },
}

impl RewriteLogEvent {
    #[allow(dead_code)]
    pub fn merge(
        source_branch: String,
        target_branch: String,
        merge_commit_sha: Option<String>,
        success: bool,
        conflicts: Vec<String>,
    ) -> Self {
        Self::Merge {
            merge: MergeEvent::new(
                source_branch,
                target_branch,
                merge_commit_sha,
                success,
                conflicts,
            ),
        }
    }

    pub fn merge_squash(event: MergeSquashEvent) -> Self {
        Self::MergeSquash {
            merge_squash: event,
        }
    }

    #[allow(dead_code)]
    pub fn rebase_interactive(event: RebaseInteractiveEvent) -> Self {
        Self::RebaseInteractive {
            rebase_interactive: event,
        }
    }

    #[allow(dead_code)]
    pub fn rebase(event: RebaseEvent) -> Self {
        Self::Rebase { rebase: event }
    }

    #[allow(dead_code)]
    pub fn cherry_pick(event: CherryPickEvent) -> Self {
        Self::CherryPick { cherry_pick: event }
    }

    #[allow(dead_code)]
    pub fn revert_mixed(event: RevertMixedEvent) -> Self {
        Self::RevertMixed {
            revert_mixed: event,
        }
    }

    #[allow(dead_code)]
    pub fn reset_soft(event: ResetSoftEvent) -> Self {
        Self::ResetSoft { reset_soft: event }
    }

    #[allow(dead_code)]
    pub fn reset_hard(event: ResetHardEvent) -> Self {
        Self::ResetHard { reset_hard: event }
    }

    pub fn commit_amend(original_commit: String, amended_commit_sha: String) -> Self {
        Self::CommitAmend {
            commit_amend: CommitAmendEvent::new(original_commit, amended_commit_sha),
        }
    }

    pub fn commit(base_commit: Option<String>, commit_sha: String) -> Self {
        Self::Commit {
            commit: CommitEvent::new(base_commit, commit_sha),
        }
    }

    #[allow(dead_code)]
    pub fn stash(event: StashEvent) -> Self {
        Self::Stash { stash: event }
    }

    #[allow(dead_code)]
    pub fn authorship_logs_synced(event: AuthorshipLogsSyncedEvent) -> Self {
        Self::AuthorshipLogsSynced {
            authorship_logs_synced: event,
        }
    }
}

/// Simple case classes - no timestamps, git already has that data
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeEvent {
    pub source_branch: String,
    pub target_branch: String,
    pub merge_commit_sha: Option<String>,
    pub success: bool,
    pub conflicts: Vec<String>,
}

impl MergeEvent {
    #[allow(dead_code)]
    pub fn new(
        source_branch: String,
        target_branch: String,
        merge_commit_sha: Option<String>,
        success: bool,
        conflicts: Vec<String>,
    ) -> Self {
        Self {
            source_branch,
            target_branch,
            merge_commit_sha,
            success,
            conflicts,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergeSquashEvent {
    pub source_branch: String,
    pub source_head: String,
    pub base_branch: String,
    pub base_head: String,
}

impl MergeSquashEvent {
    pub fn new(
        source_branch: String,
        source_head: String,
        base_branch: String,
        base_head: String,
    ) -> Self {
        Self {
            source_branch,
            source_head,
            base_branch,
            base_head,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebaseInteractiveEvent {
    pub base_commit: String,
    pub commit_count: u32,
    pub state: RebaseState,
    pub original_commits: Vec<String>,
    pub new_commits: Vec<String>,
}

impl RebaseInteractiveEvent {
    #[allow(dead_code)]
    pub fn new(
        base_commit: String,
        commit_count: u32,
        state: RebaseState,
        original_commits: Vec<String>,
        new_commits: Vec<String>,
    ) -> Self {
        Self {
            base_commit,
            commit_count,
            state,
            original_commits,
            new_commits,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RebaseEvent {
    pub base_commit: String,
    pub state: RebaseState,
    pub commit_count: u32,
    pub current_commit: Option<String>,
    pub original_commits: Vec<String>,
    pub new_commits: Vec<String>,
}

impl RebaseEvent {
    #[allow(dead_code)]
    pub fn new(
        base_commit: String,
        state: RebaseState,
        commit_count: u32,
        current_commit: Option<String>,
        original_commits: Vec<String>,
        new_commits: Vec<String>,
    ) -> Self {
        Self {
            base_commit,
            state,
            commit_count,
            current_commit,
            original_commits,
            new_commits,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CherryPickEvent {
    pub source_commit: String,
    pub target_branch: String,
    pub new_commit_sha: Option<String>,
    pub success: bool,
    pub conflicts: Vec<String>,
}

impl CherryPickEvent {
    #[allow(dead_code)]
    pub fn new(
        source_commit: String,
        target_branch: String,
        new_commit_sha: Option<String>,
        success: bool,
        conflicts: Vec<String>,
    ) -> Self {
        Self {
            source_commit,
            target_branch,
            new_commit_sha,
            success,
            conflicts,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevertMixedEvent {
    pub reverted_commit: String,
    pub success: bool,
    pub affected_files: Vec<String>,
}

impl RevertMixedEvent {
    #[allow(dead_code)]
    pub fn new(reverted_commit: String, success: bool, affected_files: Vec<String>) -> Self {
        Self {
            reverted_commit,
            success,
            affected_files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResetSoftEvent {
    pub target_commit: String,
    pub previous_head: String,
    pub success: bool,
}

impl ResetSoftEvent {
    #[allow(dead_code)]
    pub fn new(target_commit: String, previous_head: String, success: bool) -> Self {
        Self {
            target_commit,
            previous_head,
            success,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResetHardEvent {
    pub target_commit: String,
    pub previous_head: String,
    pub success: bool,
}

impl ResetHardEvent {
    #[allow(dead_code)]
    pub fn new(target_commit: String, previous_head: String, success: bool) -> Self {
        Self {
            target_commit,
            previous_head,
            success,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitAmendEvent {
    pub original_commit: String,
    pub amended_commit_sha: String,
}

impl CommitAmendEvent {
    /// Create a new CommitAmendEvent with the given parameters
    pub fn new(original_commit: String, amended_commit_sha: String) -> Self {
        Self {
            original_commit,
            amended_commit_sha,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitEvent {
    pub base_commit: Option<String>,
    pub commit_sha: String,
}

impl CommitEvent {
    /// Create a new CommitEvent with the given parameters
    pub fn new(base_commit: Option<String>, commit_sha: String) -> Self {
        Self {
            base_commit,
            commit_sha,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StashEvent {
    pub operation: StashOperation,
    pub stash_ref: Option<String>,
    pub success: bool,
    pub affected_files: Vec<String>,
}

impl StashEvent {
    #[allow(dead_code)]
    pub fn new(
        operation: StashOperation,
        stash_ref: Option<String>,
        success: bool,
        affected_files: Vec<String>,
    ) -> Self {
        Self {
            operation,
            stash_ref,
            success,
            affected_files,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorshipLogsSyncedEvent {
    pub synced: Vec<String>,
    pub origin: Vec<String>,
    pub timestamp: u64,
}

impl AuthorshipLogsSyncedEvent {
    #[allow(dead_code)]
    pub fn new(synced: Vec<String>, origin: Vec<String>) -> Self {
        Self {
            synced,
            origin,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }
}

/// Rebase state tracking
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RebaseState {
    /// Rebase started
    Start,
    /// Rebase in progress
    InProgress,
    /// Rebase continued after conflict resolution
    Continue,
    /// Rebase aborted
    Abort,
    /// Rebase completed successfully
    Complete,
}

/// Stash operation types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StashOperation {
    /// Create new stash
    Create,
    /// Apply stash (keep stash)
    Apply,
    /// Pop stash (remove after applying)
    Pop,
    /// Drop stash
    Drop,
    /// List stashes
    List,
}

/// Serialize events to JSONL format (newest events first)
#[allow(dead_code)]
pub fn serialize_events_to_jsonl(events: &[RewriteLogEvent]) -> Result<String, serde_json::Error> {
    let mut lines = Vec::new();

    // Write each event as a separate line
    for event in events {
        lines.push(serde_json::to_string(event)?);
    }

    Ok(lines.join("\n"))
}

/// Maximum number of events to keep in the rewrite log
const MAX_EVENTS: usize = 200;

/// Deserialize events from JSONL format, skipping malformed entries
pub fn deserialize_events_from_jsonl(jsonl: &str) -> Result<Vec<RewriteLogEvent>, GitAiError> {
    let mut events = Vec::new();

    for line in jsonl.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // Skip malformed entries instead of failing
        if let Ok(event) = serde_json::from_str::<RewriteLogEvent>(line) {
            events.push(event);
        }
        // Silently skip lines that don't parse - they're probably old format
    }

    // Trim to max events (keep newest, which are first due to newest-first ordering)
    if events.len() > MAX_EVENTS {
        events.truncate(MAX_EVENTS);
    }

    Ok(events)
}

/// Append a single event to JSONL file (prepends to maintain newest-first order)
pub fn append_event_to_file(
    file_path: &std::path::Path,
    new_event: RewriteLogEvent,
) -> Result<(), GitAiError> {
    // Serialize new event
    let new_event_json = serde_json::to_string(&new_event)?;

    if !file_path.exists() {
        // File doesn't exist - create it with just the new event
        std::fs::write(file_path, format!("{}\n", new_event_json))?;
        return Ok(());
    }

    // Read existing content
    let existing_content = std::fs::read_to_string(file_path)?;

    if existing_content.trim().is_empty() {
        // Empty file - just write the new event
        std::fs::write(file_path, format!("{}\n", new_event_json))?;
        return Ok(());
    }

    // Parse existing events (this will trim to MAX_EVENTS and skip malformed entries)
    let existing_events = deserialize_events_from_jsonl(&existing_content)?;

    // Create new content with new event first (newest-first order)
    let mut lines = vec![new_event_json];
    for event in existing_events {
        lines.push(serde_json::to_string(&event)?);
    }

    // Trim to max events (new event + existing events)
    if lines.len() > MAX_EVENTS {
        lines.truncate(MAX_EVENTS);
    }

    // Write back to file
    std::fs::write(file_path, lines.join("\n"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_event_serialization() {
        let event = RewriteLogEvent::merge(
            "feature-branch".to_string(),
            "main".to_string(),
            Some("abc123def456".to_string()),
            true,
            vec![],
        );

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: RewriteLogEvent = serde_json::from_str(&json).unwrap();

        match deserialized {
            RewriteLogEvent::Merge { merge } => {
                assert_eq!(merge.source_branch, "feature-branch");
                assert_eq!(merge.target_branch, "main");
                assert_eq!(merge.merge_commit_sha, Some("abc123def456".to_string()));
                assert!(merge.success);
                assert!(merge.conflicts.is_empty());
            }
            _ => panic!("Expected Merge event"),
        }
    }

    #[test]
    fn test_events_jsonl_serialization() {
        let event1 = RewriteLogEvent::merge(
            "feature".to_string(),
            "main".to_string(),
            Some("abc123".to_string()),
            true,
            vec![],
        );

        let event2 = RewriteLogEvent::cherry_pick(CherryPickEvent::new(
            "def456".to_string(),
            "main".to_string(),
            Some("ghi789".to_string()),
            true,
            vec![],
        ));

        let events = vec![event1.clone(), event2.clone()];
        let jsonl = serialize_events_to_jsonl(&events).unwrap();
        let deserialized = deserialize_events_from_jsonl(&jsonl).unwrap();

        println!("JSON L: {}", jsonl);

        assert_eq!(deserialized.len(), 2);

        match &deserialized[0] {
            RewriteLogEvent::Merge { merge } => {
                assert_eq!(merge.source_branch, "feature");
            }
            _ => panic!("Expected Merge event"),
        }

        match &deserialized[1] {
            RewriteLogEvent::CherryPick { cherry_pick } => {
                assert_eq!(cherry_pick.source_commit, "def456");
            }
            _ => panic!("Expected CherryPick event"),
        }
    }

    #[test]
    fn test_commit_amend_event_serialization() {
        let event =
            RewriteLogEvent::commit_amend("abc123def456".to_string(), "def456ghi789".to_string());

        let json = serde_json::to_string(&event).unwrap();
        println!("Serialized CommitAmend: {}", json);

        // Should serialize as {"commit_amend":{"original_commit":"abc123def456","amended_commit_sha":"def456ghi789"}}
        assert!(json.contains("\"commit_amend\""));
        assert!(json.contains("\"original_commit\":\"abc123def456\""));
        assert!(json.contains("\"amended_commit_sha\":\"def456ghi789\""));

        let deserialized: RewriteLogEvent = serde_json::from_str(&json).unwrap();

        match deserialized {
            RewriteLogEvent::CommitAmend { commit_amend } => {
                assert_eq!(commit_amend.original_commit, "abc123def456");
                assert_eq!(commit_amend.amended_commit_sha, "def456ghi789");
            }
            _ => panic!("Expected CommitAmend event"),
        }
    }

    #[test]
    fn test_append_event_to_jsonl() {
        let event1 = RewriteLogEvent::merge(
            "feature".to_string(),
            "main".to_string(),
            Some("abc123".to_string()),
            true,
            vec![],
        );

        let event2 = RewriteLogEvent::cherry_pick(CherryPickEvent::new(
            "def456".to_string(),
            "main".to_string(),
            Some("ghi789".to_string()),
            true,
            vec![],
        ));

        let initial_jsonl = serialize_events_to_jsonl(&[event1.clone()]).unwrap();
        // Test with temp file
        let temp_file = std::env::temp_dir().join("test_rewrite_log.jsonl");
        std::fs::write(&temp_file, &initial_jsonl).unwrap();
        append_event_to_file(&temp_file, event2.clone()).unwrap();
        let updated_jsonl = std::fs::read_to_string(&temp_file).unwrap();
        let deserialized = deserialize_events_from_jsonl(&updated_jsonl).unwrap();

        assert_eq!(deserialized.len(), 2);
        // event2 should be first (newest) since it was appended
        match &deserialized[0] {
            RewriteLogEvent::CherryPick { cherry_pick } => {
                assert_eq!(cherry_pick.source_commit, "def456");
            }
            _ => panic!("Expected CherryPick event"),
        }

        match &deserialized[1] {
            RewriteLogEvent::Merge { merge } => {
                assert_eq!(merge.source_branch, "feature");
            }
            _ => panic!("Expected Merge event"),
        }
    }
}

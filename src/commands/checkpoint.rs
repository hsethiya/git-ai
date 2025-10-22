use crate::authorship::attribution_tracker::{Attribution, AttributionTracker};
use crate::authorship::working_log::CheckpointKind;
use crate::authorship::working_log::{Checkpoint, WorkingLogEntry};
use crate::commands::blame::GitAiBlameOptions;
use crate::commands::checkpoint_agent::agent_presets::AgentRunResult;
use crate::error::GitAiError;
use crate::git::repo_storage::{PersistedWorkingLog, RepoStorage};
use crate::git::repository::Repository;
use crate::git::status::{EntryKind, StatusCode};
use crate::utils::{Timer, debug_log};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn run(
    repo: &Repository,
    author: &str,
    kind: CheckpointKind,
    show_working_log: bool,
    reset: bool,
    quiet: bool,
    agent_run_result: Option<AgentRunResult>,
) -> Result<(usize, usize, usize), GitAiError> {
    let total_timer = Timer::default();
    // Robustly handle zero-commit repos
    let base_commit = match repo.head() {
        Ok(head) => match head.target() {
            Ok(oid) => oid,
            Err(_) => "initial".to_string(),
        },
        Err(_) => "initial".to_string(),
    };

    // Cannot run checkpoint on bare repositories
    if repo.workdir().is_err() {
        eprintln!("Cannot run checkpoint on bare repositories");
        return Err(GitAiError::Generic(
            "Cannot run checkpoint on bare repositories".to_string(),
        ));
    }

    // Initialize the new storage system
    let repo_storage = RepoStorage::for_repo_path(repo.path());
    let working_log = repo_storage.working_log_for_base_commit(&base_commit);

    // Get the current timestamp in milliseconds since the Unix epoch
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // Extract edited filepaths from agent_run_result if available
    // For human checkpoints, use will_edit_filepaths to narrow git status scope
    // For AI checkpoints, use edited_filepaths
    // Filter out paths outside the repository to prevent git call crashes
    let mut filtered_pathspec: Option<Vec<String>> = None;
    let pathspec_filter = agent_run_result.as_ref().and_then(|result| {
        let paths = if result.checkpoint_kind == CheckpointKind::Human {
            result.will_edit_filepaths.as_ref()
        } else {
            result.edited_filepaths.as_ref()
        };

        paths.and_then(|p| {
            let repo_workdir = repo.workdir().ok()?;
            let filtered: Vec<String> = p
                .iter()
                .filter_map(|path| {
                    // Check if path is absolute and outside repo
                    if std::path::Path::new(path).is_absolute() {
                        // For absolute paths, check if they start with repo_workdir
                        if !std::path::Path::new(path).starts_with(&repo_workdir) {
                            return None;
                        }
                    } else {
                        // For relative paths, join with workdir and canonicalize to check
                        let joined = repo_workdir.join(path);
                        // Try to canonicalize to resolve .. and . components
                        if let Ok(canonical) = joined.canonicalize() {
                            if !canonical.starts_with(&repo_workdir) {
                                return None;
                            }
                        } else {
                            // If we can't canonicalize (file doesn't exist), check the joined path
                            // Convert both to canonical form if possible, otherwise use as-is
                            let normalized_joined = joined.components().fold(
                                std::path::PathBuf::new(),
                                |mut acc, component| {
                                    match component {
                                        std::path::Component::ParentDir => {
                                            acc.pop();
                                        }
                                        std::path::Component::CurDir => {}
                                        _ => acc.push(component),
                                    }
                                    acc
                                },
                            );
                            if !normalized_joined.starts_with(&repo_workdir) {
                                return None;
                            }
                        }
                    }
                    Some(path.clone())
                })
                .collect();

            if filtered.is_empty() {
                None
            } else {
                filtered_pathspec = Some(filtered);
                filtered_pathspec.as_ref()
            }
        })
    });

    let end_get_files_clock = Timer::default().start_quiet("checkpoint: get tracked files");
    let files = get_all_tracked_files(repo, &base_commit, &working_log, pathspec_filter)?;
    let get_files_duration = end_get_files_clock();
    Timer::default().print_duration("checkpoint: get tracked files", get_files_duration);
    let mut checkpoints = if reset {
        // If reset flag is set, start with an empty working log
        working_log.reset_working_log()?;
        Vec::new()
    } else {
        working_log.read_all_checkpoints()?
    };

    if show_working_log {
        if checkpoints.is_empty() {
            debug_log("No working log entries found.");
        } else {
            debug_log("Working Log Entries:");
            debug_log(&format!("{}", "=".repeat(80)));
            for (i, checkpoint) in checkpoints.iter().enumerate() {
                debug_log(&format!("Checkpoint {}", i + 1));
                debug_log(&format!("  Diff: {}", checkpoint.diff));
                debug_log(&format!("  Author: {}", checkpoint.author));
                debug_log(&format!(
                    "  Agent ID: {}",
                    checkpoint
                        .agent_id
                        .as_ref()
                        .map(|id| id.tool.clone())
                        .unwrap_or_default()
                ));

                // Display first user message from transcript if available
                if let Some(transcript) = &checkpoint.transcript {
                    if let Some(first_message) = transcript.messages().first() {
                        if let crate::authorship::transcript::Message::User { text, .. } =
                            first_message
                        {
                            let agent_info = checkpoint
                                .agent_id
                                .as_ref()
                                .map(|id| format!(" (Agent: {})", id.tool))
                                .unwrap_or_default();
                            let message_count = transcript.messages().len();
                            debug_log(&format!(
                                "  First message{} ({} messages): {}",
                                agent_info, message_count, text
                            ));
                        }
                    }
                }

                debug_log("  Entries:");
                for entry in &checkpoint.entries {
                    debug_log(&format!("    File: {}", entry.file));
                    debug_log(&format!("    Blob SHA: {}", entry.blob_sha));
                    debug_log(&format!(
                        "    Line Attributions: {:?}",
                        entry.line_attributions
                    ));
                    debug_log(&format!("    Attributions: {:?}", entry.attributions));
                }
                debug_log("");
            }
        }
        Timer::default().print_duration("checkpoint: total", total_timer.epoch.elapsed());
        return Ok((0, files.len(), checkpoints.len()));
    }

    // Save current file states and get content hashes
    let end_save_states_clock = Timer::default().start_quiet("checkpoint: persist file versions");
    let file_content_hashes = save_current_file_states(&working_log, &files)?;
    let save_states_duration = end_save_states_clock();
    Timer::default().print_duration("checkpoint: persist file versions", save_states_duration);

    // Order file hashes by key and create a hash of the ordered hashes
    let mut ordered_hashes: Vec<_> = file_content_hashes.iter().collect();
    ordered_hashes.sort_by_key(|(file_path, _)| *file_path);

    let mut combined_hasher = Sha256::new();
    for (file_path, hash) in ordered_hashes {
        combined_hasher.update(file_path.as_bytes());
        combined_hasher.update(hash.as_bytes());
    }
    let combined_hash = format!("{:x}", combined_hasher.finalize());

    // If this is not the first checkpoint, diff against the last saved state
    let end_entries_clock = Timer::default().start_quiet("checkpoint: compute entries");
    let entries = if checkpoints.is_empty() || reset {
        // First checkpoint or reset - diff against base commit
        get_initial_checkpoint_entries(
            kind,
            repo,
            &files,
            &base_commit,
            &file_content_hashes,
            agent_run_result.as_ref(),
            ts,
        )?
    } else {
        // Subsequent checkpoint - diff against last saved state
        get_subsequent_checkpoint_entries(
            kind,
            &working_log,
            &files,
            &file_content_hashes,
            &checkpoints,
            agent_run_result.as_ref(),
            ts,
        )?
    };
    let entries_duration = end_entries_clock();
    Timer::default().print_duration("checkpoint: compute entries", entries_duration);

    // Skip adding checkpoint if there are no changes
    if !entries.is_empty() {
        let mut checkpoint = Checkpoint::new(
            kind.clone(),
            combined_hash.clone(),
            author.to_string(),
            entries.clone(),
        );

        // Compute and set line stats
        let end_stats_clock = Timer::default().start_quiet("checkpoint: compute line stats");
        checkpoint.line_stats = compute_line_stats(repo, &working_log, &files, &checkpoints, kind)?;
        let stats_duration = end_stats_clock();
        Timer::default().print_duration("checkpoint: compute line stats", stats_duration);

        // Set transcript and agent_id if provided and not a human checkpoint
        if kind != CheckpointKind::Human
            && let Some(agent_run) = &agent_run_result
        {
            checkpoint.transcript = Some(agent_run.transcript.clone().unwrap_or_default());
            checkpoint.agent_id = Some(agent_run.agent_id.clone());
        }

        // Append checkpoint to the working log
        let end_append_clock = Timer::default().start_quiet("checkpoint: append working log");
        working_log.append_checkpoint(&checkpoint)?;
        let append_duration = end_append_clock();
        Timer::default().print_duration("checkpoint: append working log", append_duration);
        checkpoints.push(checkpoint);
    }

    let agent_tool = if kind != CheckpointKind::Human
        && let Some(agent_run_result) = &agent_run_result
    {
        Some(agent_run_result.agent_id.tool.as_str())
    } else {
        None
    };

    // Print summary with new format
    if reset {
        debug_log("Working log reset. Starting fresh checkpoint.");
    }

    let label = if entries.len() > 1 {
        "checkpoint"
    } else {
        "commit"
    };

    if !quiet {
        let log_author = agent_tool.unwrap_or(author);
        // Only count files that actually have checkpoint entries to avoid confusion.
        // Files that were previously checkpointed but have no new changes won't have entries.
        let files_with_entries = entries.len();
        let total_uncommitted_files = files.len();

        if files_with_entries == total_uncommitted_files {
            // All files with changes got entries
            eprintln!(
                "{} {} changed {} file(s) that have changed since the last {}",
                kind.to_str(),
                log_author,
                files_with_entries,
                label
            );
        } else {
            // Some files were already checkpointed
            eprintln!(
                "{} {} changed {} of the {} file(s) that have changed since the last {} ({} already checkpointed)",
                kind.to_str(),
                log_author,
                files_with_entries,
                total_uncommitted_files,
                label,
                total_uncommitted_files - files_with_entries
            );
        }
    }

    // Return the requested values: (entries_len, files_len, working_log_len)
    Timer::default().print_duration("checkpoint: total", total_timer.epoch.elapsed());
    Ok((entries.len(), files.len(), checkpoints.len()))
}

fn get_all_files(
    repo: &Repository,
    edited_filepaths: Option<&Vec<String>>,
) -> Result<Vec<String>, GitAiError> {
    let mut files = Vec::new();

    // Convert edited_filepaths to HashSet for git status if provided
    let pathspec = edited_filepaths.map(|paths| {
        use std::collections::HashSet;
        paths.iter().cloned().collect::<HashSet<String>>()
    });

    // Use porcelain v2 format to get status
    let statuses = repo.status(pathspec.as_ref())?;

    for entry in statuses {
        // Skip ignored files
        if entry.kind == EntryKind::Ignored {
            continue;
        }

        // Skip unmerged/conflicted files - we'll track them once the conflict is resolved
        if entry.kind == EntryKind::Unmerged {
            continue;
        }

        // Include files that have any change (staged or unstaged) or are untracked
        let has_change = entry.staged != StatusCode::Unmodified
            || entry.unstaged != StatusCode::Unmodified
            || entry.kind == EntryKind::Untracked;

        if has_change {
            // For deleted files, check if they were text files in HEAD
            let is_deleted =
                entry.staged == StatusCode::Deleted || entry.unstaged == StatusCode::Deleted;

            let is_text = if is_deleted {
                is_text_file_in_head(repo, &entry.path)
            } else {
                is_text_file(repo, &entry.path)
            };

            if is_text {
                files.push(entry.path.clone());
            }
        }
    }

    Ok(files)
}

/// Get all files that should be tracked, including those from previous checkpoints
fn get_all_tracked_files(
    repo: &Repository,
    _base_commit: &str,
    working_log: &PersistedWorkingLog,
    edited_filepaths: Option<&Vec<String>>,
) -> Result<Vec<String>, GitAiError> {
    let mut files = get_all_files(repo, edited_filepaths)?;

    // Also include files that were in previous checkpoints but might not show up in git status
    // This ensures we track deletions when files return to their original state
    if let Ok(working_log_data) = working_log.read_all_checkpoints() {
        for checkpoint in &working_log_data {
            for entry in &checkpoint.entries {
                if !files.contains(&entry.file) {
                    // Check if it's a text file before adding
                    if is_text_file(repo, &entry.file) {
                        files.push(entry.file.clone());
                    }
                }
            }
        }
    }

    Ok(files)
}

fn save_current_file_states(
    working_log: &PersistedWorkingLog,
    files: &[String],
) -> Result<HashMap<String, String>, GitAiError> {
    let mut file_content_hashes = HashMap::new();

    for file_path in files {
        let abs_path = working_log.repo_root.join(file_path);
        let content = if abs_path.exists() {
            // Read file as bytes first, then convert to string with UTF-8 lossy conversion
            match std::fs::read(&abs_path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                Err(_) => String::new(), // If we can't read the file, treat as empty
            }
        } else {
            String::new()
        };

        // Persist the file content and get the content hash
        let content_hash = working_log.persist_file_version(&content)?;
        file_content_hashes.insert(file_path.clone(), content_hash);
    }

    Ok(file_content_hashes)
}

fn get_initial_checkpoint_entries(
    kind: CheckpointKind,
    repo: &Repository,
    files: &[String],
    _base_commit: &str,
    file_content_hashes: &HashMap<String, String>,
    agent_run_result: Option<&AgentRunResult>,
    ts: u128,
) -> Result<Vec<WorkingLogEntry>, GitAiError> {
    let mut entries = Vec::new();

    // Determine author_id based on checkpoint kind and agent_id
    let author_id = if kind != CheckpointKind::Human {
        // For AI checkpoints, use session hash
        agent_run_result
            .map(|result| {
                crate::authorship::authorship_log_serialization::generate_short_hash(
                    &result.agent_id.id,
                    &result.agent_id.tool,
                )
            })
            .unwrap_or_else(|| kind.to_str())
    } else {
        // For human checkpoints, use checkpoint kind string
        kind.to_str()
    };

    // Diff working directory against HEAD tree for each file
    let head_commit = repo
        .head()
        .ok()
        .and_then(|h| h.target().ok())
        .and_then(|oid| repo.find_commit(oid).ok());
    let head_commit_sha = head_commit.as_ref().map(|c| c.id().to_string());
    let head_tree = head_commit.as_ref().and_then(|c| c.tree().ok());

    for file_path in files {
        let repo_workdir = repo.workdir().unwrap();
        let abs_path = repo_workdir.join(file_path);

        // Previous content from HEAD tree if present, otherwise empty
        let previous_content = if let Some(tree) = &head_tree {
            match tree.get_path(std::path::Path::new(file_path)) {
                Ok(entry) => {
                    if let Ok(blob) = repo.find_blob(entry.id()) {
                        let blob_content = blob.content()?;
                        String::from_utf8_lossy(&blob_content).to_string()
                    } else {
                        String::new()
                    }
                }
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        // Current content from filesystem
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

        if current_content == previous_content {
            // No changes, no need to add entries
            continue;
        }

        // Get the previous line attributions from ai blame
        let mut ai_blame_opts = GitAiBlameOptions::default();
        ai_blame_opts.no_output = true;
        ai_blame_opts.return_human_authors_as_human = true;
        ai_blame_opts.use_prompt_hashes_as_names = true;
        ai_blame_opts.newest_commit = head_commit_sha.clone();
        let ai_blame = repo.blame(file_path, &ai_blame_opts);
        let mut prev_line_attributions = Vec::new();
        if let Ok((blames, _)) = ai_blame {
            for (line, author) in blames {
                if author == CheckpointKind::Human.to_str() {
                    continue;
                }
                prev_line_attributions.push(
                    crate::authorship::attribution_tracker::LineAttribution {
                        start_line: line,
                        end_line: line,
                        author_id: author.clone(),
                    },
                );
            }
        }
        // Convert any line attributions to character attributions
        let prev_attributions =
            crate::authorship::attribution_tracker::line_attributions_to_attributions(
                &prev_line_attributions,
                &previous_content,
                ts,
            );

        // Get the blob SHA for this file from the pre-computed hashes
        let blob_sha = file_content_hashes
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        let entry = make_entry_for_file(
            file_path,
            &blob_sha,
            &author_id,
            &previous_content,
            &prev_attributions,
            &current_content,
            ts,
        )?;
        entries.push(entry);
    }

    Ok(entries)
}

fn get_subsequent_checkpoint_entries(
    kind: CheckpointKind,
    working_log: &PersistedWorkingLog,
    files: &[String],
    file_content_hashes: &HashMap<String, String>,
    previous_checkpoints: &Vec<Checkpoint>,
    agent_run_result: Option<&AgentRunResult>,
    ts: u128,
) -> Result<Vec<WorkingLogEntry>, GitAiError> {
    let mut entries = Vec::new();

    // Determine author_id based on checkpoint kind and agent_id
    let author_id = if kind != CheckpointKind::Human {
        // For AI checkpoints, use session hash
        agent_run_result
            .map(|result| {
                crate::authorship::authorship_log_serialization::generate_short_hash(
                    &result.agent_id.id,
                    &result.agent_id.tool,
                )
            })
            .unwrap_or_else(|| kind.to_str())
    } else {
        // For human checkpoints, use checkpoint kind string
        kind.to_str()
    };

    // Build a map of file path -> (blob_sha, attributions) by iterating through previous checkpoints to get the latest
    let mut previous_file_hashes_with_attributions: HashMap<String, (String, Vec<Attribution>)> =
        HashMap::new();
    for checkpoint in previous_checkpoints {
        for entry in &checkpoint.entries {
            previous_file_hashes_with_attributions.insert(
                entry.file.clone(),
                (entry.blob_sha.clone(), entry.attributions.clone()),
            );
        }
    }

    for file_path in files {
        let abs_path = working_log.repo_root.join(file_path);

        // Read current content directly from the file system
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

        // Read the previous content from the blob storage using the previous checkpoint's blob_sha
        let (previous_content, prev_attributions) = if let Some((prev_content_hash, prev_attrs)) =
            previous_file_hashes_with_attributions.get(file_path)
        {
            (
                working_log
                    .get_file_version(prev_content_hash)
                    .unwrap_or_default(),
                prev_attrs.clone(),
            )
        } else {
            (String::new(), Vec::new()) // No previous version, treat as empty
        };

        if current_content == previous_content {
            // No changes, no need to add entries
            continue;
        }

        // Get the blob SHA for this file from the pre-computed hashes
        let blob_sha = file_content_hashes
            .get(file_path)
            .cloned()
            .unwrap_or_default();

        let entry = make_entry_for_file(
            file_path,
            &blob_sha,
            &author_id,
            &previous_content,
            &prev_attributions,
            &current_content,
            ts,
        )?;
        entries.push(entry);
    }

    Ok(entries)
}

fn make_entry_for_file(
    file_path: &str,
    blob_sha: &str,
    author_id: &str,
    previous_content: &str,
    previous_attributions: &Vec<Attribution>,
    content: &str,
    ts: u128,
) -> Result<WorkingLogEntry, GitAiError> {
    let tracker = AttributionTracker::new();
    let filled_in_prev_attributions = tracker.attribute_unattributed_ranges(
        previous_content,
        previous_attributions,
        &CheckpointKind::Human.to_str(),
        ts - 1,
    );
    let new_attributions = tracker.update_attributions(
        previous_content,
        content,
        &filled_in_prev_attributions,
        author_id,
        ts,
    )?;
    // TODO Consider discarding any "uncontentious" attributions for the human author. Any human attributions that do not share a line with any other author's attributions can be discarded.
    // let filtered_attributions = crate::authorship::attribution_tracker::discard_uncontentious_attributions_for_author(&new_attributions, &CheckpointKind::Human.to_str());
    let line_attributions =
        crate::authorship::attribution_tracker::attributions_to_line_attributions(
            &new_attributions,
            content,
        );
    Ok(WorkingLogEntry::new(
        file_path.to_string(),
        blob_sha.to_string(),
        new_attributions,
        line_attributions,
    ))
}

/// Compute line statistics by diffing files against their previous versions
fn compute_line_stats(
    repo: &Repository,
    working_log: &PersistedWorkingLog,
    files: &[String],
    previous_checkpoints: &[Checkpoint],
    kind: CheckpointKind,
) -> Result<crate::authorship::working_log::CheckpointLineStats, GitAiError> {
    // Start with previous checkpoint's stats (if exists)
    let mut stats = previous_checkpoints
        .last()
        .map(|cp| cp.line_stats.clone())
        .unwrap_or_default();

    // Build a map of file path -> most recent blob_sha
    let mut previous_file_hashes: HashMap<String, String> = HashMap::new();
    for checkpoint in previous_checkpoints {
        for entry in &checkpoint.entries {
            previous_file_hashes.insert(entry.file.clone(), entry.blob_sha.clone());
        }
    }

    // Count added/deleted lines for each file in this checkpoint
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;

    for file_path in files {
        let abs_path = working_log.repo_root.join(file_path);
        let current_content = std::fs::read_to_string(&abs_path).unwrap_or_else(|_| String::new());

        // Get previous content
        let previous_content = if let Some(prev_hash) = previous_file_hashes.get(file_path) {
            working_log.get_file_version(prev_hash).unwrap_or_default()
        } else {
            // No previous version, try to get from HEAD
            let head_commit = repo
                .head()
                .ok()
                .and_then(|h| h.target().ok())
                .and_then(|oid| repo.find_commit(oid).ok());
            let head_tree = head_commit.as_ref().and_then(|c| c.tree().ok());

            if let Some(tree) = head_tree {
                match tree.get_path(std::path::Path::new(file_path)) {
                    Ok(entry) => {
                        if let Ok(blob) = repo.find_blob(entry.id()) {
                            let blob_content = blob.content().unwrap_or_default();
                            String::from_utf8_lossy(&blob_content).to_string()
                        } else {
                            String::new()
                        }
                    }
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        };

        // Use TextDiff to count line changes
        let diff = TextDiff::from_lines(&previous_content, &current_content);

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => {
                    total_additions += change.value().lines().count() as u32;
                }
                ChangeTag::Delete => {
                    total_deletions += change.value().lines().count() as u32;
                }
                ChangeTag::Equal => {}
            }
        }
    }

    // Accumulate based on checkpoint kind
    match kind {
        CheckpointKind::Human => {
            stats.human_additions += total_additions;
            stats.human_deletions += total_deletions;
        }
        CheckpointKind::AiAgent => {
            stats.ai_agent_additions += total_additions;
            stats.ai_agent_deletions += total_deletions;
        }
        CheckpointKind::AiTab => {
            stats.ai_tab_additions += total_additions;
            stats.ai_tab_deletions += total_deletions;
        }
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_utils::TmpRepo;

    #[test]
    fn test_checkpoint_with_staged_changes() {
        // Create a repo with an initial commit
        let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Make changes to the file
        file.append("New line added by user\n").unwrap();

        // Note: TmpFile.append() automatically stages changes (see write_to_disk in test_utils)
        // So at this point, the file has staged changes

        // Run checkpoint - it should track the changes even though they're staged
        let (entries_len, files_len, _checkpoints_len) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        // The bug: when changes are staged, entries_len is 0 instead of 1
        assert_eq!(files_len, 1, "Should have 1 file with changes");
        assert_eq!(
            entries_len, 1,
            "Should have 1 file entry in checkpoint (staged changes should be tracked)"
        );
    }

    #[test]
    fn test_checkpoint_with_unstaged_changes() {
        // Create a repo with an initial commit
        let (tmp_repo, file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Make changes to the file BUT keep them unstaged
        // We need to manually write to the file without staging
        let file_path = file.path();
        let mut current_content = std::fs::read_to_string(&file_path).unwrap();
        current_content.push_str("New line added by user\n");
        std::fs::write(&file_path, current_content).unwrap();

        // Run checkpoint - it should track the unstaged changes
        let (entries_len, files_len, _checkpoints_len) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        // This should work correctly
        assert_eq!(files_len, 1, "Should have 1 file with changes");
        assert_eq!(entries_len, 1, "Should have 1 file entry in checkpoint");
    }

    #[test]
    fn test_checkpoint_with_staged_changes_after_previous_checkpoint() {
        // Create a repo with an initial commit
        let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Make first changes and checkpoint
        file.append("First change\n").unwrap();
        let (entries_len_1, files_len_1, _) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        assert_eq!(
            files_len_1, 1,
            "First checkpoint: should have 1 file with changes"
        );
        assert_eq!(
            entries_len_1, 1,
            "First checkpoint: should have 1 file entry"
        );

        // Make second changes - these are already staged by append()
        file.append("Second change\n").unwrap();

        // Run checkpoint again - it should track the staged changes even after a previous checkpoint
        let (entries_len_2, files_len_2, _) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        // The bug might show up here
        println!(
            "Second checkpoint: entries_len={}, files_len={}",
            entries_len_2, files_len_2
        );
        assert_eq!(
            files_len_2, 1,
            "Second checkpoint: should have 1 file with changes"
        );
        assert_eq!(
            entries_len_2, 1,
            "Second checkpoint: should have 1 file entry in checkpoint (staged changes should be tracked)"
        );
    }

    #[test]
    fn test_checkpoint_with_only_staged_no_unstaged_changes() {
        use std::fs;

        // Create a repo with an initial commit
        let (tmp_repo, file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Get the file path
        let file_path = file.path();
        let filename = file.filename();

        // Manually modify the file (bypassing TmpFile's automatic staging)
        let mut content = fs::read_to_string(&file_path).unwrap();
        content.push_str("New line for staging test\n");
        fs::write(&file_path, &content).unwrap();

        // Now manually stage it using git (this is what "git add" does)
        tmp_repo.stage_file(filename).unwrap();

        // At this point: HEAD has old content, index has new content, workdir has new content
        // And unstaged should be "Unmodified" because workdir == index

        // Now run checkpoint
        let (entries_len, files_len, _checkpoints_len) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        println!(
            "Checkpoint result: entries_len={}, files_len={}",
            entries_len, files_len
        );

        // This should work: we should see 1 file with 1 entry
        assert_eq!(files_len, 1, "Should detect 1 file with staged changes");
        assert_eq!(
            entries_len, 1,
            "Should track the staged changes in checkpoint"
        );
    }

    #[test]
    fn test_checkpoint_then_stage_then_checkpoint_again() {
        use std::fs;

        // Create a repo with an initial commit
        let (tmp_repo, file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Get the file path
        let file_path = file.path();
        let filename = file.filename();

        // Step 1: Manually modify the file WITHOUT staging
        let mut content = fs::read_to_string(&file_path).unwrap();
        content.push_str("New line added\n");
        fs::write(&file_path, &content).unwrap();

        // Step 2: Checkpoint the unstaged changes
        let (entries_len_1, files_len_1, _) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        println!(
            "First checkpoint (unstaged): entries_len={}, files_len={}",
            entries_len_1, files_len_1
        );
        assert_eq!(files_len_1, 1, "First checkpoint: should detect 1 file");
        assert_eq!(entries_len_1, 1, "First checkpoint: should create 1 entry");

        // Step 3: Now stage the file (without making any new changes)
        tmp_repo.stage_file(filename).unwrap();

        // Step 4: Try to checkpoint again - the file is now staged but content hasn't changed
        let (entries_len_2, files_len_2, _) =
            tmp_repo.trigger_checkpoint_with_author("Aidan").unwrap();

        println!(
            "Second checkpoint (staged, no new changes): entries_len={}, files_len={}",
            entries_len_2, files_len_2
        );

        // After the fix: The checkpoint correctly recognizes that the file was already checkpointed
        // and doesn't create a duplicate entry. The improved message clarifies this to the user:
        // "changed 0 of the 1 file(s) that have changed since the last commit (1 already checkpointed)"
        assert_eq!(files_len_2, 1, "Second checkpoint: file is still staged");
        assert_eq!(
            entries_len_2, 0,
            "Second checkpoint: no NEW changes, so no new entry (already checkpointed)"
        );
    }

    #[test]
    fn test_checkpoint_skips_conflicted_files() {
        // Create a repo with an initial commit
        let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Get the current branch name (whatever the default is)
        let base_branch = tmp_repo.current_branch().unwrap();

        // Create a branch and make different changes on each branch to create a conflict
        tmp_repo.create_branch("feature-branch").unwrap();

        // On feature branch, modify the file
        file.append("Feature branch change\n").unwrap();
        tmp_repo
            .trigger_checkpoint_with_author("FeatureUser")
            .unwrap();
        tmp_repo.commit_with_message("Feature commit").unwrap();

        // Switch back to base branch and make conflicting changes
        tmp_repo.switch_branch(&base_branch).unwrap();
        file.append("Main branch change\n").unwrap();
        tmp_repo.trigger_checkpoint_with_author("MainUser").unwrap();
        tmp_repo.commit_with_message("Main commit").unwrap();

        // Attempt to merge feature-branch into base branch - this should create a conflict
        let has_conflicts = tmp_repo.merge_with_conflicts("feature-branch").unwrap();
        assert!(has_conflicts, "Should have merge conflicts");

        // Try to checkpoint while there are conflicts
        let (entries_len, files_len, _) = tmp_repo.trigger_checkpoint_with_author("Human").unwrap();

        // Checkpoint should skip conflicted files
        assert_eq!(
            files_len, 0,
            "Should have 0 files (conflicted file should be skipped)"
        );
        assert_eq!(
            entries_len, 0,
            "Should have 0 entries (conflicted file should be skipped)"
        );
    }

    #[test]
    fn test_checkpoint_with_paths_outside_repo() {
        use crate::authorship::transcript::AiTranscript;
        use crate::authorship::working_log::AgentId;
        use crate::commands::checkpoint_agent::agent_presets::AgentRunResult;

        // Create a repo with an initial commit
        let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Make changes to the file
        file.append("New line added\n").unwrap();

        // Create agent run result with paths outside the repo
        let agent_run_result = AgentRunResult {
            agent_id: AgentId {
                tool: "test_tool".to_string(),
                id: "test_session".to_string(),
                model: "test_model".to_string(),
            },
            transcript: Some(AiTranscript { messages: vec![] }),
            checkpoint_kind: CheckpointKind::AiAgent,
            repo_working_dir: None,
            edited_filepaths: Some(vec![
                "/tmp/outside_file.txt".to_string(),
                "../outside_parent.txt".to_string(),
                file.filename().to_string(), // This one is valid
            ]),
            will_edit_filepaths: None,
        };

        // Run checkpoint - should not crash even with paths outside repo
        let result =
            tmp_repo.trigger_checkpoint_with_agent_result("test_user", Some(agent_run_result));

        // Should succeed without crashing
        assert!(
            result.is_ok(),
            "Checkpoint should succeed even with paths outside repo: {:?}",
            result.err()
        );

        let (entries_len, files_len, _) = result.unwrap();
        // Should only process the valid file
        assert_eq!(files_len, 1, "Should process 1 valid file");
        assert_eq!(entries_len, 1, "Should create 1 entry");
    }

    #[test]
    fn test_checkpoint_works_after_conflict_resolution_maintains_authorship() {
        // Create a repo with an initial commit
        let (tmp_repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();

        // Get the current branch name (whatever the default is)
        let base_branch = tmp_repo.current_branch().unwrap();

        // Checkpoint initial state to track the base authorship
        let file_path = file.path();
        let initial_content = std::fs::read_to_string(&file_path).unwrap();
        println!("Initial content:\n{}", initial_content);

        // Create a branch and make changes
        tmp_repo.create_branch("feature-branch").unwrap();
        file.append("Feature line 1\n").unwrap();
        file.append("Feature line 2\n").unwrap();
        tmp_repo.trigger_checkpoint_with_author("AI_Agent").unwrap();
        tmp_repo.commit_with_message("Feature commit").unwrap();

        // Switch back to base branch and make conflicting changes
        tmp_repo.switch_branch(&base_branch).unwrap();
        file.append("Main line 1\n").unwrap();
        file.append("Main line 2\n").unwrap();
        tmp_repo.trigger_checkpoint_with_author("Human").unwrap();
        tmp_repo.commit_with_message("Main commit").unwrap();

        // Attempt to merge feature-branch into base branch - this should create a conflict
        let has_conflicts = tmp_repo.merge_with_conflicts("feature-branch").unwrap();
        assert!(has_conflicts, "Should have merge conflicts");

        // While there are conflicts, checkpoint should skip the file
        let (entries_len_conflict, files_len_conflict, _) =
            tmp_repo.trigger_checkpoint_with_author("Human").unwrap();
        assert_eq!(
            files_len_conflict, 0,
            "Should skip conflicted files during conflict"
        );
        assert_eq!(
            entries_len_conflict, 0,
            "Should not create entries for conflicted files"
        );

        // Resolve the conflict by choosing "ours" (base branch)
        tmp_repo.resolve_conflict(file.filename(), "ours").unwrap();

        // Verify content to ensure the resolution was applied correctly
        let resolved_content = std::fs::read_to_string(&file_path).unwrap();
        println!("Resolved content after resolution:\n{}", resolved_content);
        assert!(
            resolved_content.contains("Main line 1"),
            "Should contain base branch content (we chose 'ours')"
        );
        assert!(
            resolved_content.contains("Main line 2"),
            "Should contain base branch content (we chose 'ours')"
        );
        assert!(
            !resolved_content.contains("Feature line 1"),
            "Should not contain feature branch content (we chose 'ours')"
        );

        // After resolution, make additional changes to test that checkpointing works again
        file.append("Post-resolution line 1\n").unwrap();
        file.append("Post-resolution line 2\n").unwrap();

        // Now checkpoint should work and track the new changes
        let (entries_len_after, files_len_after, _) =
            tmp_repo.trigger_checkpoint_with_author("Human").unwrap();

        println!(
            "After resolution and new changes: entries_len={}, files_len={}",
            entries_len_after, files_len_after
        );

        // The file should be tracked with the new changes
        assert_eq!(
            files_len_after, 1,
            "Should detect 1 file with new changes after conflict resolution"
        );
        assert_eq!(
            entries_len_after, 1,
            "Should create 1 entry for new changes after conflict resolution"
        );
    }
}

fn is_text_file(repo: &Repository, path: &str) -> bool {
    let repo_workdir = repo.workdir().unwrap();
    let abs_path = repo_workdir.join(path);

    if let Ok(metadata) = std::fs::metadata(&abs_path) {
        if !metadata.is_file() {
            return false;
        }
    } else {
        return false; // If metadata can't be read, treat as non-text
    }

    if let Ok(content) = std::fs::read(&abs_path) {
        // Consider a file text if it contains no null bytes
        !content.contains(&0)
    } else {
        false
    }
}

fn is_text_file_in_head(repo: &Repository, path: &str) -> bool {
    // For deleted files, check if they were text files in HEAD
    let head_commit = match repo
        .head()
        .ok()
        .and_then(|h| h.target().ok())
        .and_then(|oid| repo.find_commit(oid).ok())
    {
        Some(commit) => commit,
        None => return false,
    };

    let head_tree = match head_commit.tree().ok() {
        Some(tree) => tree,
        None => return false,
    };

    match head_tree.get_path(std::path::Path::new(path)) {
        Ok(entry) => {
            if let Ok(blob) = repo.find_blob(entry.id()) {
                // Consider a file text if it contains no null bytes
                let blob_content = match blob.content() {
                    Ok(content) => content,
                    Err(_) => return false,
                };
                !blob_content.contains(&0)
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

use crate::{
    commands::hooks::commit_hooks,
    git::{cli_parser::ParsedGitInvocation, repository::Repository},
    utils::debug_log,
};

pub fn pre_reset_hook(_parsed_args: &ParsedGitInvocation, repository: &mut Repository) {
    // Capture HEAD before reset happens
    repository.require_pre_command_head();
}

pub fn post_reset_hook(parsed_args: &ParsedGitInvocation, repository: &mut Repository) {
    // Extract tree-ish (what we're resetting TO)
    let tree_ish = extract_tree_ish(parsed_args);

    // Extract pathspecs
    let pathspecs = extract_pathspecs(parsed_args).unwrap_or_else(|e| {
        debug_log(&format!("Failed to extract pathspecs: {}", e));
        Vec::new()
    });

    debug_log(&format!(
        "Reset: tree-ish='{}', pathspecs={:?}",
        tree_ish, pathspecs
    ));

    // Get old HEAD (before reset) from pre-command hook
    let old_head_sha = match &repository.pre_command_base_commit {
        Some(sha) => sha.clone(),
        None => {
            debug_log("No pre-command head captured, skipping authorship handling");
            return;
        }
    };

    // Get new HEAD (after reset)
    let new_head_sha = match repository.head().ok().and_then(|h| h.target().ok()) {
        Some(sha) => sha,
        None => {
            debug_log("No HEAD after reset, skipping authorship handling");
            return;
        }
    };

    // Resolve tree-ish to commit SHA
    let target_commit_sha = match resolve_tree_ish_to_commit(repository, &tree_ish) {
        Ok(sha) => sha,
        Err(e) => {
            debug_log(&format!("Failed to resolve tree-ish '{}': {}", tree_ish, e));
            return;
        }
    };

    // Get human author
    let human_author = commit_hooks::get_commit_default_author(repository, &[]);

    // Handle different reset modes
    // Note: Git does not allow --soft or --hard with pathspecs
    if parsed_args.has_command_flag("--hard") {
        handle_reset_hard(repository, &old_head_sha, &target_commit_sha);
    } else if parsed_args.has_command_flag("--soft")
        || parsed_args.has_command_flag("--mixed")
        || parsed_args.has_command_flag("--merge")
        || !has_reset_mode_flag(parsed_args)
    // default is --mixed
    {
        if !pathspecs.is_empty() {
            // Pathspec reset: HEAD doesn't move, but specific files are reset
            handle_reset_pathspec_preserve_working_dir(
                repository,
                &old_head_sha,
                &target_commit_sha,
                &new_head_sha,
                &human_author,
                &pathspecs,
            );
        } else {
            // Regular reset: HEAD moves
            handle_reset_preserve_working_dir(
                repository,
                &old_head_sha,
                &target_commit_sha,
                &new_head_sha,
                &human_author,
            );
        }
    }
    // --keep: no-op (git aborts if uncommitted changes exist)
}

/// Handle --hard reset: delete working log since all uncommitted work is discarded
fn handle_reset_hard(repository: &Repository, old_head_sha: &str, _target_commit_sha: &str) {
    // Delete working log for old HEAD - all uncommitted work is gone
    let _ = repository
        .storage
        .delete_working_log_for_base_commit(old_head_sha);

    debug_log(&format!(
        "Reset --hard: deleted working log for {}",
        old_head_sha
    ));
}

/// Handle --soft, --mixed, --merge: preserve working directory and reconstruct working log
fn handle_reset_preserve_working_dir(
    repository: &Repository,
    old_head_sha: &str,
    target_commit_sha: &str,
    new_head_sha: &str,
    human_author: &str,
) {
    // Sanity check: new HEAD should equal target after reset
    if new_head_sha != target_commit_sha {
        debug_log(&format!(
            "Warning: new HEAD ({}) != target commit ({})",
            new_head_sha, target_commit_sha
        ));
    }

    // No-op if resetting to same commit
    if old_head_sha == target_commit_sha {
        debug_log("Reset to same commit, no authorship changes needed");
        return;
    }

    // Check direction: are we resetting backward or forward?
    let is_backward = is_ancestor(repository, target_commit_sha, old_head_sha);

    if !is_backward {
        // Forward reset or unrelated history - treat as no-op for authorship
        // The commits we're "gaining" already have their authorship logs
        debug_log("Reset forward or to unrelated commit, no reconstruction needed");

        // Still need to delete old working log since the base commit changed
        let _ = repository
            .storage
            .delete_working_log_for_base_commit(old_head_sha);

        return;
    }

    // Backward reset: need to reconstruct working log
    match crate::authorship::rebase_authorship::reconstruct_working_log_after_reset(
        repository,
        target_commit_sha,
        old_head_sha,
        human_author,
    ) {
        Ok(_) => {
            debug_log(&format!(
                "✓ Successfully reconstructed working log after reset to {}",
                target_commit_sha
            ));
        }
        Err(e) => {
            debug_log(&format!(
                "Failed to reconstruct working log after reset: {}",
                e
            ));
        }
    }
}

/// Handle --soft, --mixed, --merge with pathspecs: preserve working directory
/// and reconstruct working log for affected files only
fn handle_reset_pathspec_preserve_working_dir(
    repository: &Repository,
    old_head_sha: &str,
    target_commit_sha: &str,
    new_head_sha: &str, // Should equal old_head_sha for pathspec resets
    human_author: &str,
    pathspecs: &[String],
) {
    debug_log(&format!(
        "Handling pathspec reset: old_head={}, target={}, pathspecs={:?}",
        old_head_sha, target_commit_sha, pathspecs
    ));

    // For pathspec resets, HEAD doesn't move
    if old_head_sha != new_head_sha {
        debug_log(&format!(
            "Warning: pathspec reset but HEAD moved from {} to {}",
            old_head_sha, new_head_sha
        ));
    }

    // Backup existing working log for HEAD (non-pathspec files)
    let working_log = repository.storage.working_log_for_base_commit(old_head_sha);
    let existing_checkpoints = working_log.read_all_checkpoints().unwrap_or_default();

    // Filter existing checkpoints to keep only non-pathspec files
    let mut non_pathspec_checkpoints = Vec::new();
    for mut checkpoint in existing_checkpoints {
        checkpoint.entries.retain(|entry| {
            !pathspecs
                .iter()
                .any(|pathspec| entry.file == *pathspec || entry.file.starts_with(pathspec))
        });
        if !checkpoint.entries.is_empty() {
            non_pathspec_checkpoints.push(checkpoint);
        }
    }

    // Run the 3-way merge reconstruction using handle_reset_preserve_working_dir
    // This will create a working log for target_commit_sha
    handle_reset_preserve_working_dir(
        repository,
        old_head_sha,
        target_commit_sha,
        target_commit_sha, // Pretend HEAD moved to target
        human_author,
    );

    // Now read the working log created for target_commit_sha
    let target_working_log = repository
        .storage
        .working_log_for_base_commit(target_commit_sha);
    let target_checkpoints = target_working_log
        .read_all_checkpoints()
        .unwrap_or_default();

    // Filter target checkpoints to only include pathspec files
    let mut pathspec_checkpoints = Vec::new();
    for mut checkpoint in target_checkpoints {
        checkpoint.entries.retain(|entry| {
            pathspecs
                .iter()
                .any(|pathspec| entry.file == *pathspec || entry.file.starts_with(pathspec))
        });
        if !checkpoint.entries.is_empty() {
            pathspec_checkpoints.push(checkpoint);
        }
    }

    // Merge the two sets of checkpoints: non-pathspec from old + pathspec from new
    let pathspec_count = pathspec_checkpoints.len();
    let non_pathspec_count = non_pathspec_checkpoints.len();
    let mut merged_checkpoints = non_pathspec_checkpoints;
    merged_checkpoints.extend(pathspec_checkpoints);

    // Save merged working log for HEAD (which hasn't moved)
    let head_working_log = repository.storage.working_log_for_base_commit(new_head_sha);
    let _ = head_working_log.reset_working_log();
    for checkpoint in merged_checkpoints {
        let _ = head_working_log.append_checkpoint(&checkpoint);
    }

    // Clean up the temporary working log for target_commit_sha (unless it's the same as HEAD)
    if target_commit_sha != new_head_sha {
        let _ = repository
            .storage
            .delete_working_log_for_base_commit(target_commit_sha);
    }

    debug_log(&format!(
        "✓ Updated working log for pathspec reset: {} pathspec checkpoints, {} non-pathspec checkpoints preserved",
        pathspec_count, non_pathspec_count
    ));
}

/// Resolve tree-ish to commit SHA
fn resolve_tree_ish_to_commit(
    repository: &Repository,
    tree_ish: &str,
) -> Result<String, crate::error::GitAiError> {
    repository
        .revparse_single(tree_ish)
        .and_then(|obj| obj.peel_to_commit())
        .map(|commit| commit.id().to_string())
}

/// Check if 'ancestor' is an ancestor of 'descendant'
fn is_ancestor(repository: &Repository, ancestor: &str, descendant: &str) -> bool {
    let mut args = repository.global_args_for_exec();
    args.push("merge-base".to_string());
    args.push("--is-ancestor".to_string());
    args.push(ancestor.to_string());
    args.push(descendant.to_string());

    crate::git::repository::exec_git(&args).is_ok()
}

/// Extract the tree-ish argument from git reset command
/// Returns "HEAD" by default if no tree-ish is provided
fn extract_tree_ish(parsed_args: &ParsedGitInvocation) -> String {
    // For reset with mode flags (--hard, --soft, --mixed, etc.),
    // the first positional arg is the commit/tree-ish
    // For reset with pathspecs, the first positional arg before -- is the tree-ish

    // Get the first positional argument
    if let Some(first_pos) = parsed_args.pos_command(0) {
        // Check if it looks like a ref/commit (not a file path)
        // Common indicators: contains ^, ~, /, or looks like a SHA
        // For simplicity, we'll consider the first positional as tree-ish
        // unless we're in pathspec mode (which we detect by presence of multiple args or --)

        // If there are pathspecs from file, first arg is tree-ish
        if has_pathspec_from_file(parsed_args) {
            return first_pos;
        }

        // Check for -- separator in command args
        if parsed_args.command_args.contains(&"--".to_string()) {
            // Find position of --
            if let Some(sep_pos) = parsed_args.command_args.iter().position(|a| a == "--") {
                // Get first positional arg before --
                let mut pos_count = 0;
                for (i, arg) in parsed_args.command_args.iter().enumerate() {
                    if i >= sep_pos {
                        break;
                    }
                    if !arg.starts_with('-') {
                        if pos_count == 0 {
                            return arg.clone();
                        }
                        pos_count += 1;
                    }
                }
            }
        }

        // Check if there's a second positional arg
        // If yes, first is tree-ish, rest are pathspecs
        // If no, and we have mode flags, it's the commit
        if parsed_args.pos_command(1).is_some() {
            return first_pos;
        }

        // Single positional arg with mode flag means it's the commit
        if has_reset_mode_flag(parsed_args) {
            return first_pos;
        }

        // Otherwise, might be a pathspec or tree-ish
        // Default to treating it as tree-ish for now
        return first_pos;
    }

    // No positional args, default to HEAD
    "HEAD".to_string()
}

/// Extract pathspecs from command line or file
fn extract_pathspecs(parsed_args: &ParsedGitInvocation) -> Result<Vec<String>, std::io::Error> {
    // Check for --pathspec-from-file flag
    if let Some(file_path) = get_pathspec_from_file_path(parsed_args) {
        return read_pathspecs_from_file(&file_path, is_pathspec_nul(parsed_args));
    }

    // Extract from command line arguments
    let mut pathspecs = Vec::new();
    let mut found_separator = false;
    let mut skip_first_positional = false;

    // Determine if we should skip the first positional (it's the tree-ish)
    if has_reset_mode_flag(parsed_args) || parsed_args.pos_command(1).is_some() {
        skip_first_positional = true;
    }

    let mut positional_count = 0;
    for arg in &parsed_args.command_args {
        if arg == "--" {
            found_separator = true;
            continue;
        }

        if found_separator {
            // Everything after -- is a pathspec
            pathspecs.push(arg.clone());
        } else if !arg.starts_with('-') {
            // Positional argument
            if skip_first_positional && positional_count == 0 {
                positional_count += 1;
                continue;
            }
            positional_count += 1;
            pathspecs.push(arg.clone());
        }
    }

    Ok(pathspecs)
}

/// Check if --pathspec-from-file is present and return the file path
fn get_pathspec_from_file_path(parsed_args: &ParsedGitInvocation) -> Option<String> {
    for arg in &parsed_args.command_args {
        if let Some(path) = arg.strip_prefix("--pathspec-from-file=") {
            return Some(path.to_string());
        }
        if arg == "--pathspec-from-file" {
            // Next arg should be the file path
            if let Some(idx) = parsed_args.command_args.iter().position(|a| a == arg) {
                if idx + 1 < parsed_args.command_args.len() {
                    return Some(parsed_args.command_args[idx + 1].clone());
                }
            }
        }
    }
    None
}

/// Check if --pathspec-file-nul is present
fn is_pathspec_nul(parsed_args: &ParsedGitInvocation) -> bool {
    parsed_args.has_command_flag("--pathspec-file-nul")
}

/// Read pathspecs from a file or stdin
fn read_pathspecs_from_file(
    file_path: &str,
    nul_separated: bool,
) -> Result<Vec<String>, std::io::Error> {
    use std::io::Read;

    let content = if file_path == "-" {
        // Read from stdin
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        buffer
    } else {
        // Read from file
        std::fs::read_to_string(file_path)?
    };

    let pathspecs: Vec<String> = if nul_separated {
        content
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    } else {
        content
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    };

    Ok(pathspecs)
}

/// Check if reset has a mode flag (--hard, --soft, --mixed, --merge, --keep)
fn has_reset_mode_flag(parsed_args: &ParsedGitInvocation) -> bool {
    parsed_args.has_command_flag("--hard")
        || parsed_args.has_command_flag("--soft")
        || parsed_args.has_command_flag("--mixed")
        || parsed_args.has_command_flag("--merge")
        || parsed_args.has_command_flag("--keep")
}

/// Check if pathspec-from-file is present
fn has_pathspec_from_file(parsed_args: &ParsedGitInvocation) -> bool {
    get_pathspec_from_file_path(parsed_args).is_some()
}

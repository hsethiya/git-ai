mod repos;

use git_ai::authorship::attribution_tracker::LineAttribution;
use git_ai::authorship::authorship_log::PromptRecord;
use git_ai::authorship::transcript::Message;
use git_ai::authorship::working_log::AgentId;
use insta::assert_debug_snapshot;
use regex::Regex;
use repos::test_repo::TestRepo;
use std::collections::HashMap;

/// Normalize blame output for snapshot testing by replacing non-deterministic
/// elements (commit SHAs and timestamps) with placeholders
fn normalize_blame_output(blame_output: &str) -> String {
    // Replace commit SHAs (40 hex chars) with placeholder
    let re_sha = Regex::new(r"[0-9a-f]{40}|[0-9a-f]{7,}").unwrap();
    let result = re_sha.replace_all(blame_output, "COMMIT_SHA");

    // Replace timestamps (e.g., "2025-10-27 11:29:32 -0400") with placeholder
    let re_timestamp = Regex::new(r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} [\+\-]\d{4}").unwrap();
    let result = re_timestamp.replace_all(&result, "TIMESTAMP");

    result.to_string()
}

#[test]
fn test_initial_only_no_blame_data() {
    // Test that INITIAL attributions work when there's no blame data (new file case)
    let repo = TestRepo::new();

    // Create initial commit to have a HEAD
    let mut readme = repo.filename("README.md");
    readme.set_contents(vec!["# Test Repo".to_string()]);
    repo.stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Get the working log for current HEAD
    let working_log = repo.current_working_logs();

    // IMPORTANT: Write INITIAL file BEFORE making any file changes
    let mut initial_attributions = HashMap::new();
    let mut line_attrs = Vec::new();
    line_attrs.push(LineAttribution {
        start_line: 1,
        end_line: 3,
        author_id: "initial-ai-123".to_string(),
        overridden: false,
    });
    initial_attributions.insert("newfile.txt".to_string(), line_attrs);

    let mut prompts = HashMap::new();
    prompts.insert(
        "initial-ai-123".to_string(),
        PromptRecord {
            agent_id: AgentId {
                tool: "test-tool".to_string(),
                id: "session-123".to_string(),
                model: "test-model".to_string(),
            },
            human_author: None,
            messages: vec![Message::assistant("Initial attribution".to_string(), None)],
            total_additions: 0,
            total_deletions: 0,
            accepted_lines: 0,
            overriden_lines: 0,
        },
    );

    working_log
        .write_initial_attributions(initial_attributions, prompts)
        .expect("write initial attributions should succeed");

    // NOW create the new file in working directory (this will trigger checkpoint reading)
    let file_content = "line 1 from INITIAL\nline 2 from INITIAL\nline 3 from INITIAL\n";
    std::fs::write(repo.path().join("newfile.txt"), file_content)
        .expect("write file should succeed");

    // Run checkpoint - should use INITIAL attributions since there's no blame data
    repo.git_ai(&["checkpoint"])
        .expect("checkpoint should succeed");

    // Commit and verify
    let commit = repo
        .stage_all_and_commit("add newfile")
        .expect("commit should succeed");

    eprintln!(
        "Authorship log prompts: {:?}",
        commit
            .authorship_log
            .metadata
            .prompts
            .keys()
            .collect::<Vec<_>>()
    );
    eprintln!(
        "Authorship log attestations: {:?}",
        commit
            .authorship_log
            .attestations
            .iter()
            .map(|a| &a.file_path)
            .collect::<Vec<_>>()
    );

    let blame_output = repo
        .git_ai(&["blame", "newfile.txt"])
        .expect("blame should succeed");

    let normalized = normalize_blame_output(&blame_output);
    assert_debug_snapshot!(normalized);
}

#[test]
fn test_initial_wins_overlaps() {
    // Test that INITIAL attributions seed the initial state
    let repo = TestRepo::new();

    // Create initial commit to have a HEAD
    let mut readme = repo.filename("README.md");
    readme.set_contents(vec!["# Test Repo".to_string()]);
    repo.stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Get the working log for current HEAD
    let working_log = repo.current_working_logs();

    // IMPORTANT: Write INITIAL file BEFORE creating the file
    let mut initial_attributions = HashMap::new();
    let mut line_attrs = Vec::new();
    line_attrs.push(LineAttribution {
        start_line: 1,
        end_line: 2,
        author_id: "initial-override-456".to_string(),
        overridden: false,
    });
    initial_attributions.insert("example.txt".to_string(), line_attrs);

    let mut prompts = HashMap::new();
    prompts.insert(
        "initial-override-456".to_string(),
        PromptRecord {
            agent_id: AgentId {
                tool: "override-tool".to_string(),
                id: "override-session".to_string(),
                model: "override-model".to_string(),
            },
            human_author: None,
            messages: vec![Message::assistant("Override attribution".to_string(), None)],
            total_additions: 0,
            total_deletions: 0,
            accepted_lines: 0,
            overriden_lines: 0,
        },
    );

    working_log
        .write_initial_attributions(initial_attributions, prompts)
        .expect("write initial attributions should succeed");

    // NOW create the file - INITIAL will seed the checkpoint
    let file_content = "line 1\nline 2\nline 3 modified\n";
    std::fs::write(repo.path().join("example.txt"), file_content)
        .expect("write file should succeed");

    // Run checkpoint
    repo.git_ai(&["checkpoint"])
        .expect("checkpoint should succeed");

    // Commit
    repo.stage_all_and_commit("add example.txt")
        .expect("commit should succeed");

    let blame_output = repo
        .git_ai(&["blame", "example.txt"])
        .expect("blame should succeed");

    let normalized = normalize_blame_output(&blame_output);
    assert_debug_snapshot!(normalized);
}

#[test]
fn test_initial_and_blame_merge() {
    // Test that INITIAL covers some lines and blame fills in the gaps
    let repo = TestRepo::new();

    // Create initial commit to have a HEAD
    let mut readme = repo.filename("README.md");
    readme.set_contents(vec!["# Test Repo".to_string()]);
    repo.stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Get the working log for current HEAD
    let working_log = repo.current_working_logs();

    // IMPORTANT: Write INITIAL file BEFORE creating the file
    // INITIAL covers lines 1-3 and 5, blame will be used for lines 4, 6, 7
    let mut initial_attributions = HashMap::new();
    let mut line_attrs = Vec::new();
    line_attrs.push(LineAttribution {
        start_line: 1,
        end_line: 3,
        author_id: "initial-123".to_string(),
        overridden: false,
    });
    line_attrs.push(LineAttribution {
        start_line: 5,
        end_line: 5,
        author_id: "initial-456".to_string(),
        overridden: false,
    });
    initial_attributions.insert("example.txt".to_string(), line_attrs);

    let mut prompts = HashMap::new();
    prompts.insert(
        "initial-123".to_string(),
        PromptRecord {
            agent_id: AgentId {
                tool: "tool1".to_string(),
                id: "session1".to_string(),
                model: "model1".to_string(),
            },
            human_author: None,
            messages: vec![Message::assistant("Attribution 123".to_string(), None)],
            total_additions: 0,
            total_deletions: 0,
            accepted_lines: 0,
            overriden_lines: 0,
        },
    );
    prompts.insert(
        "initial-456".to_string(),
        PromptRecord {
            agent_id: AgentId {
                tool: "tool2".to_string(),
                id: "session2".to_string(),
                model: "model2".to_string(),
            },
            human_author: None,
            messages: vec![Message::assistant("Attribution 456".to_string(), None)],
            total_additions: 0,
            total_deletions: 0,
            accepted_lines: 0,
            overriden_lines: 0,
        },
    );

    working_log
        .write_initial_attributions(initial_attributions, prompts)
        .expect("write initial attributions should succeed");

    // NOW create the file - INITIAL will seed lines 1-3, 5; blame will be used for 4, 6, 7
    // Write directly to filesystem for direct control
    let file_content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n";
    std::fs::write(repo.path().join("example.txt"), file_content)
        .expect("write file should succeed");

    // Run checkpoint with mock_ai so lines 4, 6, 7 get mock attribution
    repo.git_ai(&["checkpoint", "mock_ai"])
        .expect("checkpoint should succeed");

    // Commit
    repo.stage_all_and_commit("add example.txt")
        .expect("commit should succeed");

    let blame_output = repo
        .git_ai(&["blame", "example.txt"])
        .expect("blame should succeed");

    let normalized = normalize_blame_output(&blame_output);
    assert_debug_snapshot!(normalized);
}

#[test]
fn test_partial_file_coverage() {
    // Test that INITIAL has data for fileA but not fileB
    let repo = TestRepo::new();

    // Create initial commit to have a HEAD
    let mut readme = repo.filename("README.md");
    readme.set_contents(vec!["# Test Repo".to_string()]);
    repo.stage_all_and_commit("initial commit")
        .expect("commit should succeed");

    // Get the working log for current HEAD
    let working_log = repo.current_working_logs();

    // IMPORTANT: Write INITIAL file AFTER creating initial commit
    let mut initial_attributions = HashMap::new();
    let mut line_attrs = Vec::new();
    line_attrs.push(LineAttribution {
        start_line: 1,
        end_line: 2,
        author_id: "initial-fileA".to_string(),
        overridden: false,
    });
    initial_attributions.insert("fileA.txt".to_string(), line_attrs);
    // Note: fileB.txt is not in INITIAL

    let mut prompts = HashMap::new();
    prompts.insert(
        "initial-fileA".to_string(),
        PromptRecord {
            agent_id: AgentId {
                tool: "toolA".to_string(),
                id: "sessionA".to_string(),
                model: "modelA".to_string(),
            },
            human_author: None,
            messages: vec![Message::assistant("FileA attribution".to_string(), None)],
            total_additions: 0,
            total_deletions: 0,
            accepted_lines: 0,
            overriden_lines: 0,
        },
    );

    working_log
        .write_initial_attributions(initial_attributions, prompts)
        .expect("write initial attributions should succeed");

    // NOW create both files - fileA gets INITIAL, fileB uses blame
    std::fs::write(repo.path().join("fileA.txt"), "line 1 in A\nline 2 in A\n")
        .expect("write file should succeed");
    std::fs::write(repo.path().join("fileB.txt"), "line 1 in B\nline 2 in B\n")
        .expect("write file should succeed");

    // Run checkpoint with mock_ai so fileB gets mock attribution
    repo.git_ai(&["checkpoint", "mock_ai"])
        .expect("checkpoint should succeed");

    // Commit
    repo.stage_all_and_commit("add both files")
        .expect("commit should succeed");

    // Check blame for fileA - should show INITIAL attributions (toolA)
    let blame_a = repo
        .git_ai(&["blame", "fileA.txt"])
        .expect("blame should succeed");

    let normalized_a = normalize_blame_output(&blame_a);
    assert_debug_snapshot!(normalized_a);

    // Check blame for fileB - should show mock (no INITIAL, so blame is used)
    let blame_b = repo
        .git_ai(&["blame", "fileB.txt"])
        .expect("blame should succeed");
    let normalized_b = normalize_blame_output(&blame_b);
    assert_debug_snapshot!(normalized_b);
}

#[macro_use]
mod repos;
use repos::test_file::ExpectedLineExt;
use repos::test_repo::TestRepo;

#[test]
fn test_blame_after_merge_with_ai_contributions() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    // Create base file and initial commit
    file.set_contents(lines!["Base line 1", "Base line 2", "Base line 3"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // Save the default branch name before creating feature branch
    let default_branch = repo.current_branch();

    // Create a feature branch
    repo.git(&["checkout", "-b", "feature"]).unwrap();

    // Make AI changes on feature branch (insert after line 3)
    file.insert_at(3, lines!["FEATURE LINE 1".ai(), "FEATURE LINE 2".ai()]);
    repo.stage_all_and_commit("feature branch changes").unwrap();

    // Switch back to default branch and make human changes
    repo.git(&["checkout", &default_branch]).unwrap();
    file = repo.filename("test.txt"); // Reload file from default branch
    // Insert at beginning to avoid conflict with feature branch
    file.insert_at(0, lines!["MAIN LINE 1", "MAIN LINE 2"]);
    repo.stage_all_and_commit("main branch changes").unwrap();

    // Merge feature branch into default branch (should not conflict)
    repo.git(&["merge", "feature", "-m", "merge feature into main"])
        .unwrap();

    // Test blame after merge - should have both AI and human contributions
    file = repo.filename("test.txt");
    file.assert_lines_and_blame(lines![
        "MAIN LINE 1".human(),
        "MAIN LINE 2".human(),
        "Base line 1".human(),
        "Base line 2".human(),
        "Base line 3".human(),
        "FEATURE LINE 1".ai(),
        "FEATURE LINE 2".ai(),
    ]);
}

// #[test]
// fn test_blame_after_rebase_with_ai_contributions() {
//     let tmp_dir = tempdir().unwrap();
//     let repo_path = tmp_dir.path().to_path_buf();

//     // Create initial repository with base commit
//     let (mut tmp_repo, mut lines, mut alphabet) =
//         TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

//     // Create a feature branch
//     tmp_repo.create_branch("feature").unwrap();

//     // Make changes on feature branch (add lines at the end)
//     lines
//         .append("REBASE FEATURE LINE 1\nREBASE FEATURE LINE 2\n")
//         .unwrap();
//     tmp_repo.trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor")).unwrap();
//     tmp_repo
//         .commit_with_message("feature branch changes")
//         .unwrap();

//     // Switch back to the default branch and make different changes (insert in middle)
//     let default_branch = tmp_repo.get_default_branch().unwrap();
//     tmp_repo.switch_branch(&default_branch).unwrap();
//     lines
//         .insert_at(15 * 2, "REBASE MAIN LINE 1\nREBASE MAIN LINE 2\n")
//         .unwrap();
//     tmp_repo
//         .trigger_checkpoint_with_author("test_user")
//         .unwrap();
//     tmp_repo.commit_with_message("main branch changes").unwrap();

//     // Switch back to feature and rebase onto the default branch
//     tmp_repo.switch_branch("feature").unwrap();
//     let default_branch = tmp_repo.get_default_branch().unwrap();
//     tmp_repo.rebase_onto("feature", &default_branch).unwrap();

//     // Test blame after rebase
//     let blame = tmp_repo.blame_for_file(&lines, Some((30, 36))).unwrap();
//     assert_debug_snapshot!(blame);
// }

#[test]
fn test_blame_after_complex_merge_scenario() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    // Create base file and initial commit
    file.set_contents(lines!["Base line 1", "Base line 2"]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // Save the default branch name
    let default_branch = repo.current_branch();

    // Create feature-a branch
    repo.git(&["checkout", "-b", "feature-a"]).unwrap();
    file.insert_at(2, lines!["FEATURE A LINE 1".ai(), "FEATURE A LINE 2".ai()]);
    repo.stage_all_and_commit("feature a changes").unwrap();

    // Create feature-b branch (from feature-a)
    repo.git(&["checkout", "-b", "feature-b"]).unwrap();
    file.insert_at(4, lines!["FEATURE B LINE 1".ai(), "FEATURE B LINE 2".ai()]);
    repo.stage_all_and_commit("feature b changes").unwrap();

    // Switch back to default branch and make human changes
    repo.git(&["checkout", &default_branch]).unwrap();
    file = repo.filename("test.txt"); // Reload file from default branch
    // Insert at beginning to avoid conflicts
    file.insert_at(0, lines!["MAIN COMPLEX LINE 1", "MAIN COMPLEX LINE 2"]);
    repo.stage_all_and_commit("main complex changes").unwrap();

    // Merge feature-a into default branch
    repo.git(&["merge", "feature-a", "-m", "merge feature-a into main"])
        .unwrap();

    // Merge feature-b into default branch
    repo.git(&["merge", "feature-b", "-m", "merge feature-b into main"])
        .unwrap();

    // Test blame after complex merge - should have all contributions
    file = repo.filename("test.txt");
    file.assert_lines_and_blame(lines![
        "MAIN COMPLEX LINE 1".human(),
        "MAIN COMPLEX LINE 2".human(),
        "Base line 1".human(),
        "Base line 2".human(),
        "FEATURE A LINE 1".ai(),
        "FEATURE A LINE 2".ai(),
        "FEATURE B LINE 1".ai(),
        "FEATURE B LINE 2".ai(),
    ]);
}

// #[test]
// fn test_blame_after_rebase_chain() {
//     let tmp_dir = tempdir().unwrap();
//     let repo_path = tmp_dir.path().to_path_buf();

//     // Create initial repository with base commit
//     let (mut tmp_repo, mut lines, mut alphabet) =
//         TmpRepo::new_with_base_commit(repo_path.clone()).unwrap();

//     // Create a feature branch
//     tmp_repo.create_branch("feature").unwrap();

//     // Make multiple commits on feature branch
//     lines.append("REBASE CHAIN 1\n").unwrap();
//     tmp_repo.trigger_checkpoint_with_ai("Claude", Some("claude-3-sonnet"), Some("cursor")).unwrap();
//     tmp_repo.commit_with_message("feature commit 1").unwrap();

//     lines.append("REBASE CHAIN 2\n").unwrap();
//     tmp_repo.trigger_checkpoint_with_author("GPT-4").unwrap();
//     tmp_repo.commit_with_message("feature commit 2").unwrap();

//     // Switch back to the default branch and make changes
//     let default_branch = tmp_repo.get_default_branch().unwrap();
//     tmp_repo.switch_branch(&default_branch).unwrap();
//     lines.append("MAIN CHAIN 1\n").unwrap();
//     tmp_repo
//         .trigger_checkpoint_with_author("test_user")
//         .unwrap();
//     tmp_repo.commit_with_message("main commit 1").unwrap();

//     lines.append("MAIN CHAIN 2\n").unwrap();
//     tmp_repo
//         .trigger_checkpoint_with_author("test_user")
//         .unwrap();
//     tmp_repo.commit_with_message("main commit 2").unwrap();

//     // Switch back to feature and rebase onto the default branch
//     tmp_repo.switch_branch("feature").unwrap();
//     let default_branch = tmp_repo.get_default_branch().unwrap();
//     tmp_repo.rebase_onto("feature", &default_branch).unwrap();

//     // Test blame after rebase chain
//     let blame = tmp_repo.blame_for_file(&lines, None).unwrap();
//     println!("blame: {:?}", blame);
//     assert_debug_snapshot!(blame);
// }

#[test]
fn test_blame_after_merge_conflict_resolution() {
    let repo = TestRepo::new();
    let mut file = repo.filename("test.txt");

    // Create base file with multiple lines
    file.set_contents(lines![
        "Line 1", "Line 2", "Line 3", "Line 4", "Line 5", "Line 6", "Line 7", "Line 8", "Line 9",
        "Line 10",
    ]);
    repo.stage_all_and_commit("Initial commit").unwrap();

    // Save the default branch name
    let default_branch = repo.current_branch();

    // Create a feature branch
    repo.git(&["checkout", "-b", "feature"]).unwrap();

    // Make AI changes on feature branch (replace line 5)
    file.replace_at(4, "CONFLICT FEATURE VERSION".ai());
    repo.stage_all_and_commit("feature conflict changes")
        .unwrap();

    // Switch back to default branch and make conflicting human changes
    repo.git(&["checkout", &default_branch]).unwrap();
    file = repo.filename("test.txt"); // Reload file from main branch
    file.replace_at(4, "CONFLICT MAIN VERSION");
    repo.stage_all_and_commit("main conflict changes").unwrap();

    // Merge feature branch into main (conflicts will occur)
    // Git will exit with error on conflict, so we handle it
    let merge_result = repo.git(&[
        "merge",
        "feature",
        "-m",
        "merge feature with conflict resolution",
    ]);

    if merge_result.is_err() {
        // Resolve conflict by accepting main's version
        file = repo.filename("test.txt");
        file.set_contents(lines![
            "Line 1",
            "Line 2",
            "Line 3",
            "Line 4",
            "CONFLICT MAIN VERSION",
            "Line 6",
            "Line 7",
            "Line 8",
            "Line 9",
            "Line 10",
        ]);
        repo.stage_all_and_commit("merge feature with conflict resolution")
            .unwrap();
    }

    // Test blame after conflict resolution
    file = repo.filename("test.txt");
    file.assert_lines_and_blame(lines![
        "Line 1".human(),
        "Line 2".human(),
        "Line 3".human(),
        "Line 4".human(),
        "CONFLICT MAIN VERSION".human(),
        "Line 6".human(),
        "Line 7".human(),
        "Line 8".human(),
        "Line 9".human(),
        "Line 10".human(),
    ]);
}

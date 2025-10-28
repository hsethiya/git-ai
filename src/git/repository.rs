use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::rebase_authorship::rewrite_authorship_if_needed;
use crate::config;
use crate::error::GitAiError;
use crate::git::cli_parser::ParsedGitInvocation;
use crate::git::refs::{get_authorship, show_authorship_note};
use crate::git::repo_storage::RepoStorage;
use crate::git::rewrite_log::RewriteLogEvent;
use crate::git::sync_authorship::{fetch_authorship_notes, push_authorship_notes};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

pub struct Object<'a> {
    repo: &'a Repository,
    oid: String,
}

impl<'a> Object<'a> {
    pub fn id(&self) -> String {
        self.oid.clone()
    }

    // Recursively peel an object until a commit is found.
    pub fn peel_to_commit(&self) -> Result<Commit<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        args.push(format!("{}^{}", self.oid, "{commit}"));
        let output = exec_git(&args)?;
        Ok(Commit {
            repo: self.repo,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
            authorship_log: std::cell::OnceCell::new(),
        })
    }
}

#[derive(Debug)]
pub struct CommitRange<'a> {
    repo: &'a Repository,
    pub start_oid: String,
    pub end_oid: String,
    pub refname: String,
}

impl<'a> CommitRange<'a> {
    pub fn new(
        repo: &'a Repository,
        start_oid: String,
        end_oid: String,
        refname: String,
    ) -> Result<Self, GitAiError> {
        // Resolve start_oid and end_oid to actual commit SHAs
        let resolved_start = repo.revparse_single(&start_oid)?.oid;
        let resolved_end = repo.revparse_single(&end_oid)?.oid;

        Ok(Self {
            repo,
            start_oid: resolved_start,
            end_oid: resolved_end,
            refname,
        })
    }

    /// Create a new CommitRange with automatic refname inference.
    /// If refname is None, tries to find a single ref pointing to end_oid.
    /// If exactly one ref is found, uses that. Otherwise falls back to current HEAD.
    pub fn new_infer_refname(
        repo: &'a Repository,
        start_oid: String,
        end_oid: String,
        refname: Option<String>,
    ) -> Result<Self, GitAiError> {
        // Resolve start_oid and end_oid to actual commit SHAs
        let resolved_start = repo.revparse_single(&start_oid)?.oid;
        let resolved_end = repo.revparse_single(&end_oid)?.oid;

        let inferred_refname = match refname {
            Some(name) => name,
            None => {
                // Try to find refs pointing to resolved end_oid
                let mut args = repo.global_args_for_exec();
                args.push("for-each-ref".to_string());
                args.push("--points-at".to_string());
                args.push(resolved_end.clone());
                args.push("--format=%(refname)".to_string());

                let refs = match exec_git(&args) {
                    Ok(output) => {
                        let stdout = String::from_utf8(output.stdout).unwrap_or_default();
                        let refs: Vec<String> = stdout
                            .lines()
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        refs
                    }
                    Err(_) => Vec::new(),
                };

                // If exactly one ref found, use it
                if refs.len() == 1 {
                    refs[0].clone()
                } else {
                    // Fall back to current HEAD
                    match repo.head() {
                        Ok(head_ref) => head_ref.name().unwrap_or("HEAD").to_string(),
                        Err(_) => "HEAD".to_string(),
                    }
                }
            }
        };

        Ok(Self {
            repo,
            start_oid: resolved_start,
            end_oid: resolved_end,
            refname: inferred_refname,
        })
    }

    pub fn repo(&self) -> &'a Repository {
        self.repo
    }

    pub fn is_valid(&self) -> Result<(), GitAiError> {
        // Check that both commits exist
        self.repo.find_commit(self.start_oid.clone())?;
        self.repo.find_commit(self.end_oid.clone())?;

        // Check that both commits exist on the refname
        // Use git merge-base --is-ancestor <commit> <refname>
        let mut args = self.repo.global_args_for_exec();
        args.push("merge-base".to_string());
        args.push("--is-ancestor".to_string());
        args.push(self.start_oid.clone());
        args.push(self.refname.clone());

        exec_git(&args).map_err(|_| {
            GitAiError::Generic(format!(
                "Commit {} is not reachable from refname {}",
                self.start_oid, self.refname
            ))
        })?;

        let mut args = self.repo.global_args_for_exec();
        args.push("merge-base".to_string());
        args.push("--is-ancestor".to_string());
        args.push(self.end_oid.clone());
        args.push(self.refname.clone());

        exec_git(&args).map_err(|_| {
            GitAiError::Generic(format!(
                "Commit {} is not reachable from refname {}",
                self.end_oid, self.refname
            ))
        })?;

        // Check that start is an ancestor of end (direct path between them)
        let mut args = self.repo.global_args_for_exec();
        args.push("merge-base".to_string());
        args.push("--is-ancestor".to_string());
        args.push(self.start_oid.clone());
        args.push(self.end_oid.clone());

        exec_git(&args).map_err(|_| {
            GitAiError::Generic(format!(
                "Commit {} is not an ancestor of {}",
                self.start_oid, self.end_oid
            ))
        })?;

        Ok(())
    }

    pub fn length(&self) -> usize {
        // Use git rev-list --count to get the number of commits between start and end
        // Format: start_oid..end_oid means commits reachable from end_oid but not from start_oid
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-list".to_string());
        args.push("--count".to_string());
        args.push(format!("{}..{}", self.start_oid, self.end_oid));

        match exec_git(&args) {
            Ok(output) => {
                let count_str = String::from_utf8(output.stdout).unwrap_or_default();
                count_str.trim().parse().unwrap_or(0)
            }
            Err(_) => 0, // If they don't share lineage or error occurs, return 0
        }
    }
}

impl<'a> IntoIterator for CommitRange<'a> {
    type Item = Commit<'a>;
    type IntoIter = CommitRangeIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        // ie for single commit branches
        if self.start_oid == self.end_oid {
            return CommitRangeIterator {
                repo: self.repo,
                commit_oids: vec![self.end_oid.clone()],
                index: 0,
            };
        }

        // Use git rev-list to get all commits between start and end
        // Format: start_oid..end_oid means commits reachable from end_oid but not from start_oid
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-list".to_string());
        args.push(format!("{}..{}", self.start_oid, self.end_oid));

        let commit_oids: Vec<String> = match exec_git(&args) {
            Ok(output) => {
                let stdout = String::from_utf8(output.stdout).unwrap_or_default();
                stdout
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
            Err(_) => Vec::new(), // If they don't share lineage or error occurs, return empty
        };

        CommitRangeIterator {
            repo: self.repo,
            commit_oids,
            index: 0,
        }
    }
}

pub struct CommitRangeIterator<'a> {
    repo: &'a Repository,
    commit_oids: Vec<String>,
    index: usize,
}

impl<'a> Iterator for CommitRangeIterator<'a> {
    type Item = Commit<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.commit_oids.len() {
            return None;
        }
        let oid = self.commit_oids[self.index].clone();
        self.index += 1;
        Some(Commit {
            repo: self.repo,
            oid,
            authorship_log: std::cell::OnceCell::new(),
        })
    }
}

pub struct Signature<'a> {
    #[allow(dead_code)]
    repo: &'a Repository,
    name: String,
    email: String,
    time_iso8601: String,
}

pub struct Time {
    seconds: i64,
    offset_minutes: i32,
}

impl Time {
    pub fn seconds(&self) -> i64 {
        self.seconds
    }

    pub fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }
}

impl<'a> Signature<'a> {
    pub fn name(&self) -> Option<&str> {
        if self.name.is_empty() {
            None
        } else {
            Some(self.name.as_str())
        }
    }

    pub fn email(&self) -> Option<&str> {
        if self.email.is_empty() {
            None
        } else {
            Some(self.email.as_str())
        }
    }

    pub fn when(&self) -> Time {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&self.time_iso8601) {
            let seconds = dt.timestamp();
            let offset_minutes = dt.offset().local_minus_utc() / 60;
            Time {
                seconds,
                offset_minutes,
            }
        } else {
            // TODO Log error
            // Fallback to epoch if parsing fails
            Time {
                seconds: 0,
                offset_minutes: 0,
            }
        }
    }
}

pub struct Commit<'a> {
    repo: &'a Repository,
    oid: String,
    authorship_log: std::cell::OnceCell<AuthorshipLog>,
}

impl<'a> Commit<'a> {
    pub fn id(&self) -> String {
        self.oid.clone()
    }

    pub fn tree(&self) -> Result<Tree<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        args.push(format!("{}^{}", self.oid, "{tree}"));
        let output = exec_git(&args)?;
        Ok(Tree {
            repo: self.repo,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
        })
    }

    pub fn parent(&self, i: usize) -> Result<Commit<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        // libgit2 uses 0-based indexing; Git's rev syntax uses 1-based parent selectors.
        args.push(format!("{}^{}", self.oid, i + 1));
        let output = exec_git(&args)?;
        Ok(Commit {
            repo: self.repo,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
            authorship_log: std::cell::OnceCell::new(),
        })
    }

    // Return an iterator over the parents of this commit.
    pub fn parents(&self) -> Parents<'a> {
        // Use `git show -s --format=%P <oid>` to get whitespace-separated parent OIDs
        let mut args = self.repo.global_args_for_exec();
        args.push("show".to_string());
        args.push("-s".to_string());
        args.push("--format=%P".to_string());
        args.push(self.oid.clone());

        let parent_oids: Vec<String> = match exec_git(&args) {
            Ok(output) => {
                let stdout = String::from_utf8(output.stdout).unwrap_or_default();
                stdout.split_whitespace().map(|s| s.to_string()).collect()
            }
            Err(_) => Vec::new(),
        };

        Parents {
            repo: self.repo,
            parent_oids,
            index: 0,
        }
    }

    // Get the number of parents of this commit.
    // Use the parents iterator to return an iterator over all parents.
    #[allow(dead_code)]
    pub fn parent_count(&self) -> Result<usize, GitAiError> {
        Ok(self.parents().count())
    }

    // Get the short “summary” of the git commit message. The returned message is the summary of the commit, comprising the first paragraph of the message with whitespace trimmed and squashed. None may be returned if an error occurs or if the summary is not valid utf-8.
    pub fn summary(&self) -> Result<String, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("show".to_string());
        args.push("-s".to_string());
        args.push("--no-notes".to_string());
        args.push("--encoding=UTF-8".to_string());
        args.push("--format=%s".to_string());
        args.push(self.oid.clone());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Get the author of this commit.
    pub fn author(&self) -> Result<Signature<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("show".to_string());
        args.push("-s".to_string());
        args.push("--no-notes".to_string());
        args.push("--encoding=UTF-8".to_string());
        args.push("--format=%an%n%ae%n%aI".to_string());
        args.push(self.oid.clone());
        let output = exec_git(&args)?;
        let stdout = String::from_utf8(output.stdout)?;
        let mut lines = stdout.lines();
        let name = lines.next().unwrap_or("").trim().to_string();
        let email = lines.next().unwrap_or("").trim().to_string();
        let time_iso8601 = lines.next().unwrap_or("").trim().to_string();
        Ok(Signature {
            repo: self.repo,
            name,
            email,
            time_iso8601,
        })
    }

    // Get the committer of this commit.
    pub fn committer(&self) -> Result<Signature<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("show".to_string());
        args.push("-s".to_string());
        args.push("--no-notes".to_string());
        args.push("--encoding=UTF-8".to_string());
        args.push("--format=%cn%n%ce%n%cI".to_string());
        args.push(self.oid.clone());
        let output = exec_git(&args)?;
        let stdout = String::from_utf8(output.stdout)?;
        let mut lines = stdout.lines();
        let name = lines.next().unwrap_or("").trim().to_string();
        let email = lines.next().unwrap_or("").trim().to_string();
        let time_iso8601 = lines.next().unwrap_or("").trim().to_string();
        Ok(Signature {
            repo: self.repo,
            name,
            email,
            time_iso8601,
        })
    }

    // Get the commit time (i.e. committer time) of a commit.
    // The first element of the tuple is the time, in seconds, since the epoch. The second element is the offset, in minutes, of the time zone of the committer’s preferred time zone.
    pub fn time(&self) -> Result<Time, GitAiError> {
        let signature = self.committer()?;
        Ok(signature.when())
    }

    // lazy load the authorship log
    pub fn authorship(&self) -> &AuthorshipLog {
        self.authorship_log.get_or_init(|| {
            get_authorship(self.repo, self.oid.as_str()).unwrap_or_else(|| AuthorshipLog::new())
        })
    }
    pub fn authorship_uncached(&self) -> AuthorshipLog {
        get_authorship(self.repo, self.oid.as_str()).unwrap_or_else(|| AuthorshipLog::new())
    }
}

pub struct TreeEntry<'a> {
    #[allow(dead_code)]
    repo: &'a Repository,
    // Object id (SHA-1/oid) that this tree entry points to
    oid: String,
    // One of: blob, tree, commit (gitlink)
    #[allow(dead_code)]
    object_type: String,
    // File mode as provided by git ls-tree (e.g. 100644, 100755, 120000, 040000)
    #[allow(dead_code)]
    mode: String,
    // Full path relative to the root of the tree used for lookup
    #[allow(dead_code)]
    path: String,
}

impl<'a> TreeEntry<'a> {
    // Get the id of the object pointed by the entry
    pub fn id(&self) -> String {
        self.oid.clone()
    }
}

pub struct Tree<'a> {
    repo: &'a Repository,
    oid: String,
}

impl<'a> Tree<'a> {
    // Get the id of the tree
    pub fn id(&self) -> String {
        self.oid.clone()
    }

    pub fn clone(&self) -> Tree<'a> {
        Tree {
            repo: self.repo,
            oid: self.oid.clone(),
        }
    }

    // Retrieve a tree entry contained in a tree or in any of its subtrees, given its relative path.
    pub fn get_path(&self, path: &Path) -> Result<TreeEntry<'a>, GitAiError> {
        // Use `git ls-tree -z -d <tree-oid> -- <path>` to get exactly the entry for the path.
        // -z ensures NUL-terminated records; -d shows the directory itself instead of listing contents
        let mut args = self.repo.global_args_for_exec();
        args.push("ls-tree".to_string());
        args.push("-z".to_string());
        // Use recursive to locate files in nested paths and return blob entries
        args.push("-r".to_string());
        args.push(self.oid.clone());
        args.push("--".to_string());
        let path_str = path.to_string_lossy().to_string();
        args.push(path_str.clone());

        let output = exec_git(&args)?;
        let bytes = output.stdout;

        // Each record: "<mode> <type> <object>\t<file>\0"
        // We expect at most one record for an exact path query.
        let mut found_entry: Option<TreeEntry<'a>> = None;

        for chunk in bytes.split(|b| *b == 0u8) {
            if chunk.is_empty() {
                continue;
            }
            // Split metadata and path on first tab
            let mut parts = chunk.splitn(2, |b| *b == b'\t');
            let meta = parts.next().unwrap_or(&[]);
            let file_bytes = parts.next().unwrap_or(&[]);

            // Parse meta: "<mode> <type> <object>"
            let meta_str = String::from_utf8_lossy(meta);
            let mut meta_iter = meta_str.split_whitespace();
            let mode = meta_iter.next().unwrap_or("").to_string();
            let object_type = meta_iter.next().unwrap_or("").to_string();
            let oid = meta_iter.next().unwrap_or("").to_string();

            if mode.is_empty() || object_type.is_empty() || oid.is_empty() {
                continue;
            }

            let file_path = String::from_utf8_lossy(file_bytes).to_string();

            // Prefer exact path match if multiple records somehow appear
            if found_entry.is_none() || file_path == path_str {
                found_entry = Some(TreeEntry {
                    repo: self.repo,
                    oid,
                    object_type,
                    mode,
                    path: file_path,
                });
            }
        }

        match found_entry {
            Some(entry) => Ok(entry),
            None => Err(GitAiError::Generic(format!(
                "Path not found in tree: {}",
                path.to_string_lossy()
            ))),
        }
    }
}

pub struct Blob<'a> {
    repo: &'a Repository,
    oid: String,
}

impl<'a> Blob<'a> {
    #[allow(dead_code)]
    pub fn id(&self) -> String {
        self.oid.clone()
    }

    // Get the content of this blob.
    pub fn content(&self) -> Result<Vec<u8>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("cat-file".to_string());
        args.push("blob".to_string());
        args.push(self.oid.clone());
        let output = exec_git(&args)?;
        Ok(output.stdout)
    }
}

pub struct Reference<'a> {
    repo: &'a Repository,
    ref_name: String,
}

impl<'a> Reference<'a> {
    pub fn name(&self) -> Option<&str> {
        Some(&self.ref_name)
    }

    #[allow(dead_code)]
    pub fn is_branch(&self) -> bool {
        self.ref_name.starts_with("refs/heads/")
    }

    #[allow(dead_code)]
    pub fn shorthand(&self) -> Result<String, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        args.push("--abbrev-ref".to_string());
        args.push(self.ref_name.clone());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub fn target(&self) -> Result<String, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        args.push(self.ref_name.clone());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Peel a reference to a blob
    // This method recursively peels the reference until it reaches a blob.
    #[allow(dead_code)]
    pub fn peel_to_blob(&self) -> Result<Blob<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        args.push(format!("{}^{}", self.ref_name, "{blob}"));
        let output = exec_git(&args)?;
        Ok(Blob {
            repo: self.repo,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
        })
    }

    // Peel a reference to a commit This method recursively peels the reference until it reaches a commit.
    pub fn peel_to_commit(&self) -> Result<Commit<'a>, GitAiError> {
        let mut args = self.repo.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        args.push(format!("{}^{}", self.ref_name, "{commit}"));
        let output = exec_git(&args)?;
        Ok(Commit {
            repo: self.repo,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
            authorship_log: std::cell::OnceCell::new(),
        })
    }
}

pub struct Parents<'a> {
    repo: &'a Repository,
    parent_oids: Vec<String>,
    index: usize,
}

impl<'a> Iterator for Parents<'a> {
    type Item = Commit<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.parent_oids.len() {
            return None;
        }
        let oid = self.parent_oids[self.index].clone();
        self.index += 1;
        Some(Commit {
            repo: self.repo,
            oid,
            authorship_log: std::cell::OnceCell::new(),
        })
    }
}

pub struct References<'a> {
    repo: &'a Repository,
    refs: Vec<String>,
    index: usize,
}

impl<'a> Iterator for References<'a> {
    type Item = Result<Reference<'a>, GitAiError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.refs.len() {
            return None;
        }
        let ref_name = self.refs[self.index].clone();
        self.index += 1;
        Some(Ok(Reference {
            repo: self.repo,
            ref_name,
        }))
    }
}

#[derive(Debug, Clone)]
pub struct Repository {
    global_args: Vec<String>,
    git_dir: PathBuf,
    pub storage: RepoStorage,
    pub pre_command_base_commit: Option<String>,
    pub pre_command_refname: Option<String>,
    workdir_cache: OnceLock<Result<PathBuf, GitAiError>>,
}

impl Repository {
    // Util for preparing global args for execution
    pub fn global_args_for_exec(&self) -> Vec<String> {
        let mut args = self.global_args.clone();
        if !args.iter().any(|arg| arg == "--no-pager") {
            args.push("--no-pager".to_string());
        }
        args
    }

    pub fn require_pre_command_head(&mut self) {
        if self.pre_command_base_commit.is_some() || self.pre_command_refname.is_some() {
            return;
        }

        // Safely handle empty repositories
        if let Ok(head_ref) = self.head() {
            if let Ok(target) = head_ref.target() {
                let target_string = target;
                let refname = head_ref.name().map(|n| n.to_string());
                self.pre_command_base_commit = Some(target_string);
                self.pre_command_refname = refname;
            }
        }
    }

    pub fn handle_rewrite_log_event(
        &mut self,
        rewrite_log_event: RewriteLogEvent,
        commit_author: String,
        supress_output: bool,
        apply_side_effects: bool,
    ) {
        let log = self
            .storage
            .append_rewrite_event(rewrite_log_event.clone())
            .ok()
            .expect("Error writing .git/ai/rewrite_log");

        if apply_side_effects {
            match rewrite_authorship_if_needed(
                self,
                &rewrite_log_event,
                commit_author,
                &log,
                supress_output,
            ) {
                Ok(_) => (),
                Err(_) => {}
            }
        }
    }

    // Internal util to get the git object type for a given OID
    fn object_type(&self, oid: &str) -> Result<String, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("cat-file".to_string());
        args.push("-t".to_string());
        args.push(oid.to_string());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Retrieve and resolve the reference pointed at by HEAD.
    // If HEAD is a symbolic ref, return the refname (e.g., "refs/heads/main").
    // Otherwise, return "HEAD".
    pub fn head<'a>(&'a self) -> Result<Reference<'a>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("symbolic-ref".to_string());
        // args.push("-q".to_string());
        args.push("HEAD".to_string());

        let output = exec_git(&args);

        match output {
            Ok(output) if output.status.success() => {
                let refname = String::from_utf8(output.stdout)?;
                Ok(Reference {
                    repo: self,
                    ref_name: refname.trim().to_string(),
                })
            }
            _ => Ok(Reference {
                repo: self,
                ref_name: "HEAD".to_string(),
            }),
        }
    }

    // Returns the path to the .git folder for normal repositories or the repository itself for bare repositories.
    // TODO Test on bare repositories.
    pub fn path(&self) -> &Path {
        self.git_dir.as_path()
    }

    // Get the path of the working directory for this repository.
    // If this repository is bare, then None is returned.
    pub fn workdir(&self) -> Result<PathBuf, GitAiError> {
        self.workdir_cache
            .get_or_init(|| {
                let mut args = self.global_args_for_exec();
                args.push("rev-parse".to_string());
                args.push("--show-toplevel".to_string());

                let output = exec_git(&args)?;
                let git_dir_str = String::from_utf8(output.stdout)?;

                let git_dir_str = git_dir_str.trim();
                let path = PathBuf::from(git_dir_str);
                if !path.is_dir() {
                    return Err(GitAiError::Generic(format!(
                        "Git directory does not exist: {}",
                        git_dir_str
                    )));
                }

                Ok(path)
            })
            .clone()
    }

    // List all remotes for a given repository
    pub fn remotes(&self) -> Result<Vec<String>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("remote".to_string());

        let output = exec_git(&args)?;
        let remotes = String::from_utf8(output.stdout)?;
        Ok(remotes.trim().split("\n").map(|s| s.to_string()).collect())
    }

    // List all remotes with their URLs as tuples (name, url)
    pub fn remotes_with_urls(&self) -> Result<Vec<(String, String)>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("remote".to_string());
        args.push("-v".to_string());

        let output = exec_git(&args)?;
        let remotes_output = String::from_utf8(output.stdout)?;

        let mut remotes = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for line in remotes_output.trim().split("\n").filter(|s| !s.is_empty()) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let url = parts[1].to_string();
                // Only add each remote once (git remote -v shows fetch and push)
                if seen.insert(name.clone()) {
                    remotes.push((name, url));
                }
            }
        }

        Ok(remotes)
    }

    pub fn config_get_str(&self, key: &str) -> Result<Option<String>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("config".to_string());
        args.push("--get".to_string());
        args.push(key.to_string());
        match exec_git(&args) {
            Ok(output) => Ok(Some(String::from_utf8(output.stdout)?.trim().to_string())),
            Err(GitAiError::GitCliError { code: Some(1), .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    #[allow(dead_code)]
    pub fn config_set_str(&self, key: &str, value: &str) -> Result<(), GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("config".to_string());
        args.push("set".to_string());
        args.push(key.to_string());
        args.push(value.to_string());
        exec_git(&args)?;
        Ok(())
    }

    // Write an in-memory buffer to the ODB as a blob.
    // The Oid returned can in turn be passed to find_blob to get a handle to the blob.
    #[allow(dead_code)]
    pub fn blob(&self, data: &[u8]) -> Result<String, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("hash-object".to_string());
        args.push("-w".to_string());
        args.push("--stdin".to_string());
        let output = exec_git_stdin(&args, data)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Create a new direct reference. This function will return an error if a reference already exists with the given name unless force is true, in which case it will be overwritten.
    #[allow(dead_code)]
    pub fn reference<'a>(
        &'a self,
        name: &str,
        id: String,
        force: bool,
        log_message: &str,
    ) -> Result<Reference<'a>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("update-ref".to_string());
        args.push("--stdin".to_string());
        args.push("--create-reflog".to_string());
        args.push("-m".to_string());
        args.push(log_message.to_string());

        let verb = if force { "update" } else { "create" };
        let stdin_line = format!("{} {} {}\n", verb, name, id.trim());
        exec_git_stdin(&args, stdin_line.as_bytes())?;

        Ok(Reference {
            repo: self,
            ref_name: name.to_string(),
        })
    }

    pub fn remote_head(&self, remote_name: &str) -> Result<String, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("symbolic-ref".to_string());
        args.push(format!("refs/remotes/{}/HEAD", remote_name));
        args.push("--short".to_string());

        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Lookup a reference to one of the objects in a repository. Requires full ref name.
    #[allow(dead_code)]
    pub fn find_reference(&self, name: &str) -> Result<Reference<'_>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("show-ref".to_string());
        args.push("--verify".to_string());
        args.push("-s".to_string());
        args.push(name.to_string());
        exec_git(&args)?;
        Ok(Reference {
            repo: self,
            ref_name: name.to_string(),
        })
    }
    // Find a merge base between two commits
    pub fn merge_base(&self, one: String, two: String) -> Result<String, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("merge-base".to_string());
        args.push(one.to_string());
        args.push(two.to_string());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    // Merge two trees, producing an index that reflects the result of the merge. The index may be written as-is to the working directory or checked out. If the index is to be converted to a tree, the caller should resolve any conflicts that arose as part of the merge.
    pub fn merge_trees_favor_ours(
        &self,
        ancestor_tree: &Tree<'_>,
        our_tree: &Tree<'_>,
        their_tree: &Tree<'_>,
    ) -> Result<String, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("merge-tree".to_string());
        args.push("--write-tree".to_string());
        args.push(format!("--merge-base={}", ancestor_tree.oid));
        args.push("-X".to_string());
        args.push("ours".to_string());
        args.push(our_tree.oid.to_string());
        args.push(their_tree.oid.to_string());
        let output = exec_git(&args)?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub fn commit_range_on_branch(&self, branch_refname: &str) -> Result<CommitRange, GitAiError> {
        // Normalize the provided branch ref to fully qualified using rev-parse
        let fq_branch = {
            let mut rp_args = self.global_args_for_exec();
            rp_args.push("rev-parse".to_string());
            rp_args.push("--verify".to_string());
            rp_args.push("--symbolic-full-name".to_string());
            rp_args.push(branch_refname.to_string());

            match exec_git(&rp_args) {
                Ok(output) => {
                    let s = String::from_utf8(output.stdout).unwrap_or_default();
                    let s = s.trim();
                    if s.is_empty() {
                        if branch_refname.starts_with("refs/") {
                            branch_refname.to_string()
                        } else {
                            format!("refs/heads/{}", branch_refname)
                        }
                    } else {
                        s.to_string()
                    }
                }
                Err(_) => {
                    if branch_refname.starts_with("refs/") {
                        branch_refname.to_string()
                    } else {
                        format!("refs/heads/{}", branch_refname)
                    }
                }
            }
        };

        // List all local branches
        let mut refs_args = self.global_args_for_exec();
        refs_args.push("for-each-ref".to_string());
        refs_args.push("--format=%(refname)".to_string());
        refs_args.push("refs/heads".to_string());
        let refs_output = exec_git(&refs_args)?;
        let refs_str = String::from_utf8(refs_output.stdout)?;
        let mut other_branches: Vec<String> = Vec::new();

        for line in refs_str.lines() {
            let refname = line.trim();
            if refname.is_empty() {
                continue;
            }
            if refname == fq_branch {
                continue;
            }
            other_branches.push(refname.to_string());
        }

        // Build: git log --format=%H --reverse --ancestry-path <branch> --not <other branches>
        let mut log_args = self.global_args_for_exec();
        log_args.push("log".to_string());
        log_args.push("--format=%H".to_string());
        log_args.push("--reverse".to_string());
        log_args.push("--ancestry-path".to_string());
        log_args.push(branch_refname.to_string());
        if !other_branches.is_empty() {
            log_args.push("--not".to_string());
            for ob in other_branches {
                log_args.push(ob);
            }
        }

        let log_output = exec_git(&log_args).map_err(|e| {
            GitAiError::Generic(format!(
                "Failed to get commit log for {}: {:?}",
                branch_refname, e
            ))
        })?;

        let log_str = String::from_utf8(log_output.stdout)
            .map_err(|e| GitAiError::Generic(format!("Failed to parse log output: {:?}", e)))?;

        let commits: Vec<&str> = log_str.lines().filter(|line| !line.is_empty()).collect();

        if commits.is_empty() {
            return Err(GitAiError::Generic(format!(
                "No commits found on branch {} unique to this branch",
                branch_refname
            )));
        }

        let first_commit = commits.first().unwrap().to_string();
        let last_commit = commits.last().unwrap().to_string();

        Ok(CommitRange::new(
            self,
            first_commit,
            last_commit,
            branch_refname.to_string(),
        )?)
    }

    // Create new commit in the repository If the update_ref is not None, name of the reference that will be updated to point to this commit. If the reference is not direct, it will be resolved to a direct reference. Use “HEAD” to update the HEAD of the current branch and make it point to this commit. If the reference doesn’t exist yet, it will be created. If it does exist, the first parent must be the tip of this branch.
    pub fn commit(
        &self,
        update_ref: Option<&str>,
        author: &Signature<'_>,
        committer: &Signature<'_>,
        message: &str,
        tree: &Tree<'_>,
        parents: &[&Commit<'_>],
    ) -> Result<String, GitAiError> {
        // Validate identities
        let author_name = author.name().unwrap_or("").trim().to_string();
        let author_email = author.email().unwrap_or("").trim().to_string();
        let committer_name = committer.name().unwrap_or("").trim().to_string();
        let committer_email = committer.email().unwrap_or("").trim().to_string();

        if author_name.is_empty() || author_email.is_empty() {
            return Err(GitAiError::Generic(
                "Missing author name or email".to_string(),
            ));
        }
        if committer_name.is_empty() || committer_email.is_empty() {
            return Err(GitAiError::Generic(
                "Missing committer name or email".to_string(),
            ));
        }

        // Format dates as "<unix-seconds> <±HHMM>" which Git accepts
        let fmt_git_date = |t: Time| -> String {
            let seconds = t.seconds();
            let offset_min = t.offset_minutes();
            let sign = if offset_min >= 0 { '+' } else { '-' };
            let abs = offset_min.abs();
            let hh = abs / 60;
            let mm = abs % 60;
            format!("{} {}{:02}{:02}", seconds, sign, hh, mm)
        };
        let author_date = fmt_git_date(author.when());
        let committer_date = fmt_git_date(committer.when());

        // Build env for commit-tree
        let mut env: Vec<(String, String)> = Vec::new();
        env.push(("GIT_AUTHOR_NAME".to_string(), author_name));
        env.push(("GIT_AUTHOR_EMAIL".to_string(), author_email));
        env.push(("GIT_AUTHOR_DATE".to_string(), author_date));
        env.push(("GIT_COMMITTER_NAME".to_string(), committer_name));
        env.push(("GIT_COMMITTER_EMAIL".to_string(), committer_email));
        env.push(("GIT_COMMITTER_DATE".to_string(), committer_date));

        // 1) Create the commit object via commit-tree, piping message on stdin
        let mut ct_args = self.global_args_for_exec();
        ct_args.push("commit-tree".to_string());
        ct_args.push(tree.oid.clone());
        for p in parents.iter() {
            ct_args.push("-p".to_string());
            ct_args.push(p.id());
        }
        let ct_out = exec_git_stdin_with_env(&ct_args, &env, message.as_bytes())?;
        let new_commit = String::from_utf8(ct_out.stdout)?.trim().to_string();

        // 2) Optionally update a ref with CAS semantics
        if let Some(update_ref_name) = update_ref {
            // Resolve target ref (HEAD may be symbolic)
            let target_ref = if update_ref_name == "HEAD" {
                // If HEAD is symbolic this returns e.g. refs/heads/main; otherwise "HEAD"
                self.head()?.name().unwrap().to_string()
            } else {
                update_ref_name.to_string()
            };

            // Capture current tip if any: rev-parse -q --verify <target_ref>
            let mut rp_args = self.global_args_for_exec();
            rp_args.push("rev-parse".to_string());
            // rp_args.push("-q".to_string()); // For gitai, we want to see the error message if the ref doesn't exist
            rp_args.push("--verify".to_string());
            rp_args.push(target_ref.clone());

            let old_tip: Option<String> = match Command::new(config::Config::get().git_cmd())
                .args(&rp_args)
                .output()
            {
                Ok(output) if output.status.success() => {
                    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                }
                _ => None,
            };

            // Enforce first-parent matches current tip if ref exists
            if let Some(ref tip) = old_tip {
                if parents.is_empty() {
                    return Err(GitAiError::Generic(
                        "Ref exists but no parents were provided".to_string(),
                    ));
                }
                let first_parent = parents[0].id();
                if first_parent.trim() != tip {
                    return Err(GitAiError::Generic(format!(
                        "First parent ({}) != current tip ({}) of {}",
                        first_parent, tip, target_ref
                    )));
                }
            }

            // Update the ref atomically (include OLD_TIP for CAS if present)
            let mut ur_args = self.global_args_for_exec();
            ur_args.push("update-ref".to_string());
            ur_args.push("-m".to_string());
            ur_args.push(message.to_string());
            ur_args.push(target_ref.clone());
            ur_args.push(new_commit.clone());
            if let Some(tip) = old_tip {
                ur_args.push(tip);
            }
            exec_git(&ur_args)?;
        }

        Ok(new_commit)
    }

    // Find a single object, as specified by a revision string.
    pub fn revparse_single(&self, spec: &str) -> Result<Object<'_>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("rev-parse".to_string());
        // args.push("-q".to_string());
        args.push("--verify".to_string());
        args.push(spec.to_string());
        let output = exec_git(&args)?;
        Ok(Object {
            repo: self,
            oid: String::from_utf8(output.stdout)?.trim().to_string(),
        })
    }

    // Non-standard method of getting a 'default' remote
    pub fn get_default_remote(&self) -> Result<Option<String>, GitAiError> {
        let remotes = self.remotes()?;
        if remotes.len() == 0 {
            return Ok(None);
        }
        // Prefer 'origin' if it exists
        for i in 0..remotes.len() {
            if let Some(name) = remotes.get(i) {
                if name == "origin" {
                    return Ok(Some("origin".to_string()));
                }
            }
        }
        // Otherwise, just use the first remote
        Ok(remotes.get(0).map(|s| s.to_string()))
    }

    pub fn fetch_authorship<'a>(&'a self, remote_name: &str) -> Result<(), GitAiError> {
        fetch_authorship_notes(self, remote_name)
    }

    pub fn push_authorship<'a>(&'a self, remote_name: &str) -> Result<(), GitAiError> {
        push_authorship_notes(self, remote_name)
    }

    pub fn upstream_remote(&self) -> Result<Option<String>, GitAiError> {
        // Get current branch name using exec_git
        let mut args = self.global_args_for_exec();
        args.push("branch".to_string());
        args.push("--show-current".to_string());
        let output = exec_git(&args)?;
        let branch = String::from_utf8(output.stdout)?.trim().to_string();
        if branch.is_empty() {
            return Ok(None);
        }
        let config_key = format!("branch.{}.remote", branch);
        self.config_get_str(&config_key)
    }

    pub fn resolve_author_spec(&self, author_spec: &str) -> Result<Option<String>, GitAiError> {
        // Use git rev-list to find the first commit by this author pattern
        let mut args = self.global_args_for_exec();
        args.push("rev-list".to_string());
        args.push("--all".to_string());
        args.push("-i".to_string());
        args.push("--max-count=1".to_string());
        args.push(format!("--author={}", author_spec));
        let output = match exec_git(&args) {
            Ok(output) => output,
            Err(GitAiError::GitCliError { code: Some(1), .. }) => {
                // No commit found
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let commit_oid = String::from_utf8(output.stdout)?.trim().to_string();
        if commit_oid.is_empty() {
            return Ok(None);
        }

        // Now get the author name/email from that commit
        let mut show_args = self.global_args_for_exec();
        show_args.push("show".to_string());
        show_args.push("-s".to_string());
        show_args.push("--format=%an <%ae>".to_string());
        show_args.push(commit_oid);
        let show_output = exec_git(&show_args)?;
        let author_line = String::from_utf8(show_output.stdout)?.trim().to_string();
        if author_line.is_empty() {
            Ok(None)
        } else {
            Ok(Some(author_line))
        }
    }

    // Create an iterator for the repo’s references (git2-style)
    pub fn references<'a>(&'a self) -> Result<References<'a>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("for-each-ref".to_string());
        args.push("--format=%(refname)".to_string());

        let output = exec_git(&args)?;
        let stdout = String::from_utf8(output.stdout)?;
        let refs: Vec<String> = stdout
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        Ok(References {
            repo: self,
            refs,
            index: 0,
        })
    }

    // Lookup a reference to one of the commits in a repository.
    pub fn find_commit(&self, oid: String) -> Result<Commit<'_>, GitAiError> {
        let typ = self.object_type(&oid)?;
        if typ != "commit" {
            return Err(GitAiError::Generic(format!(
                "Object is not a commit: {} (type: {})",
                oid, typ
            )));
        }
        Ok(Commit {
            repo: self,
            oid,
            authorship_log: std::cell::OnceCell::new(),
        })
    }

    // Lookup a reference to one of the objects in a repository.
    pub fn find_blob(&self, oid: String) -> Result<Blob<'_>, GitAiError> {
        let typ = self.object_type(&oid)?;
        if typ != "blob" {
            return Err(GitAiError::Generic(format!(
                "Object is not a blob: {} (type: {})",
                oid, typ
            )));
        }
        Ok(Blob { repo: self, oid })
    }

    // Lookup a reference to one of the objects in a repository.
    pub fn find_tree(&self, oid: String) -> Result<Tree<'_>, GitAiError> {
        let typ = self.object_type(&oid)?;
        if typ != "tree" {
            return Err(GitAiError::Generic(format!(
                "Object is not a tree: {} (type: {})",
                oid, typ
            )));
        }
        Ok(Tree { repo: self, oid })
    }

    /// Get the content of a file at a specific commit
    /// Uses `git show <commit>:<path>` for efficient single-call retrieval
    pub fn get_file_content(
        &self,
        file_path: &str,
        commit_hash: &str,
    ) -> Result<Vec<u8>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("show".to_string());
        args.push(format!("{}:{}", commit_hash, file_path));
        let output = exec_git(&args)?;
        Ok(output.stdout)
    }

    /// List all files changed in a commit
    /// Returns a HashSet of file paths relative to the repository root
    pub fn list_commit_files(
        &self,
        commit_sha: &str,
        pathspecs: Option<&HashSet<String>>,
    ) -> Result<HashSet<String>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("diff-tree".to_string());
        args.push("--no-commit-id".to_string());
        args.push("--name-only".to_string());
        args.push("-r".to_string());

        // Find the commit to check if it has a parent
        let commit = self.find_commit(commit_sha.to_string())?;

        // For initial commits (no parent), compare against the empty tree
        if commit.parent_count()? == 0 {
            let empty_tree = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
            args.push(empty_tree.to_string());
        }

        args.push(commit_sha.to_string());

        // Add pathspecs if provided
        if let Some(paths) = pathspecs {
            args.push("--".to_string());
            for path in paths {
                args.push(path.clone());
            }
        }

        let output = exec_git(&args)?;
        let stdout = String::from_utf8(output.stdout)?;

        let files: HashSet<String> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();

        Ok(files)
    }

    /// Get added line ranges from git diff between two commits
    /// Returns a HashMap of file paths to vectors of added line numbers
    ///
    /// Uses `git diff -U0` to get unified diff with zero context lines,
    /// then parses the hunk headers to extract line numbers directly.
    /// This is much faster than fetching blobs and running TextDiff manually.
    pub fn diff_added_lines(
        &self,
        from_ref: &str,
        to_ref: &str,
        pathspecs: Option<&HashSet<String>>,
    ) -> Result<HashMap<String, Vec<u32>>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("diff".to_string());
        args.push("-U0".to_string()); // Zero context lines
        args.push("--no-color".to_string());
        args.push(from_ref.to_string());
        args.push(to_ref.to_string());

        // Add pathspecs if provided
        if let Some(paths) = pathspecs {
            args.push("--".to_string());
            for path in paths {
                args.push(path.clone());
            }
        }

        let output = exec_git(&args)?;
        let diff_output = String::from_utf8(output.stdout)?;

        parse_diff_added_lines(&diff_output)
    }

    /// Get added line ranges from git diff between a commit and the working directory
    /// Returns a HashMap of file paths to vectors of added line numbers
    ///
    /// Similar to diff_added_lines but compares against the working directory
    pub fn diff_workdir_added_lines(
        &self,
        from_ref: &str,
        pathspecs: Option<&HashSet<String>>,
    ) -> Result<HashMap<String, Vec<u32>>, GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("diff".to_string());
        args.push("-U0".to_string()); // Zero context lines
        args.push("--no-color".to_string());
        args.push(from_ref.to_string());

        // Add pathspecs if provided
        if let Some(paths) = pathspecs {
            args.push("--".to_string());
            for path in paths {
                args.push(path.clone());
            }
        }

        let output = exec_git(&args)?;
        let diff_output = String::from_utf8(output.stdout)?;

        parse_diff_added_lines(&diff_output)
    }

    pub fn fetch_branch(&self, branch_name: &str, remote_name: &str) -> Result<(), GitAiError> {
        let mut args = self.global_args_for_exec();
        args.push("fetch".to_string());
        args.push(remote_name.to_string());
        args.push(branch_name.to_string());
        exec_git(&args)?;
        Ok(())
    }
}

pub fn find_repository(global_args: &Vec<String>) -> Result<Repository, GitAiError> {
    let mut args = global_args.clone();
    args.push("rev-parse".to_string());
    args.push("--absolute-git-dir".to_string());

    let output = exec_git(&args)?;
    let git_dir_str = String::from_utf8(output.stdout)?;

    let git_dir_str = git_dir_str.trim();
    let path = PathBuf::from(git_dir_str);
    if !path.is_dir() {
        return Err(GitAiError::Generic(format!(
            "Git directory does not exist: {}",
            git_dir_str
        )));
    }

    Ok(Repository {
        global_args: global_args.clone(),
        storage: RepoStorage::for_repo_path(&path),
        git_dir: path,
        pre_command_base_commit: None,
        pre_command_refname: None,
        workdir_cache: OnceLock::new(),
    })
}

pub fn find_repository_in_path(path: &str) -> Result<Repository, GitAiError> {
    let global_args = vec!["-C".to_string(), path.to_string()];
    return find_repository(&global_args);
}

/// Helper to execute a git command
pub fn exec_git(args: &[String]) -> Result<Output, GitAiError> {
    // TODO Make sure to handle process signals, etc.
    let output = Command::new(config::Config::get().git_cmd())
        .args(args)
        .output()
        .map_err(GitAiError::IoError)?;

    if !output.status.success() {
        let code = output.status.code();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitAiError::GitCliError {
            code,
            stderr,
            args: args.to_vec(),
        });
    }

    Ok(output)
}

/// Helper to execute a git command with data provided on stdin
pub fn exec_git_stdin(args: &[String], stdin_data: &[u8]) -> Result<Output, GitAiError> {
    // TODO Make sure to handle process signals, etc.
    let mut child = Command::new(config::Config::get().git_cmd())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(GitAiError::IoError)?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if let Err(e) = stdin.write_all(stdin_data) {
            return Err(GitAiError::IoError(e));
        }
    }

    let output = child.wait_with_output().map_err(GitAiError::IoError)?;

    if !output.status.success() {
        let code = output.status.code();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitAiError::GitCliError {
            code,
            stderr,
            args: args.to_vec(),
        });
    }

    Ok(output)
}

/// Helper to execute a git command with data provided on stdin and additional environment variables
pub fn exec_git_stdin_with_env(
    args: &[String],
    env: &Vec<(String, String)>,
    stdin_data: &[u8],
) -> Result<Output, GitAiError> {
    // TODO Make sure to handle process signals, etc.
    let mut cmd = Command::new(config::Config::get().git_cmd());
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // Apply env overrides
    for (k, v) in env.iter() {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(GitAiError::IoError)?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        if let Err(e) = stdin.write_all(stdin_data) {
            return Err(GitAiError::IoError(e));
        }
    }

    let output = child.wait_with_output().map_err(GitAiError::IoError)?;

    if !output.status.success() {
        let code = output.status.code();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitAiError::GitCliError {
            code,
            stderr,
            args: args.to_vec(),
        });
    }

    Ok(output)
}

/// Parse git diff output to extract added line numbers per file
///
/// Parses unified diff format hunk headers like:
/// @@ -10,2 +15,5 @@
///
/// This means: old file line 10 (2 lines), new file line 15 (5 lines)
/// We extract the "new file" line numbers to know which lines were added.
fn parse_diff_added_lines(diff_output: &str) -> Result<HashMap<String, Vec<u32>>, GitAiError> {
    let mut result: HashMap<String, Vec<u32>> = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in diff_output.lines() {
        // Track current file being diffed
        if line.starts_with("+++ b/") {
            current_file = Some(line[6..].to_string());
        } else if line.starts_with("+++ /dev/null") {
            // File was deleted
            current_file = None;
        } else if line.starts_with("@@ ") {
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
            if let Some(ref file) = current_file {
                if let Some(added_lines) = parse_hunk_header(line) {
                    result
                        .entry(file.clone())
                        .or_insert_with(Vec::new)
                        .extend(added_lines);
                }
            }
        }
    }

    // Sort and deduplicate line numbers for each file
    for lines in result.values_mut() {
        lines.sort_unstable();
        lines.dedup();
    }

    Ok(result)
}

/// Parse a hunk header line to extract added line numbers
///
/// Format: @@ -old_start,old_count +new_start,new_count @@
/// Returns the line numbers that were added in the new file
fn parse_hunk_header(line: &str) -> Option<Vec<u32>> {
    // Find the part between @@ and @@
    let parts: Vec<&str> = line.split("@@").collect();
    if parts.len() < 2 {
        return None;
    }

    let hunk_info = parts[1].trim();

    // Split by space to get old and new ranges
    let ranges: Vec<&str> = hunk_info.split_whitespace().collect();
    if ranges.len() < 2 {
        return None;
    }

    // Parse the new file range (starts with '+')
    let new_range = ranges
        .iter()
        .find(|r| r.starts_with('+'))?
        .trim_start_matches('+');

    // Parse "start,count" or just "start"
    let new_parts: Vec<&str> = new_range.split(',').collect();
    let start: u32 = new_parts[0].parse().ok()?;
    let count: u32 = if new_parts.len() > 1 {
        new_parts[1].parse().ok()?
    } else {
        1 // If no count specified, it's 1 line
    };

    // If count is 0, no lines were added (only deleted)
    if count == 0 {
        return Some(Vec::new());
    }

    // Generate all line numbers in the range
    let lines: Vec<u32> = (start..start + count).collect();
    Some(lines)
}

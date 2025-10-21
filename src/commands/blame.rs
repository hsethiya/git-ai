use crate::authorship::authorship_log::PromptRecord;
use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::authorship::working_log::CheckpointKind;
use crate::error::GitAiError;
use crate::git::refs::get_reference_as_authorship_log_v3;
use crate::git::repository::Repository;
use crate::git::repository::exec_git;
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use std::collections::HashMap;
use std::fs;
use std::io::{self, IsTerminal, Write};

#[derive(Debug, Clone)]
pub struct BlameHunk {
    /// Line range [start, end] (inclusive) - current line numbers in the file
    pub range: (u32, u32),
    /// Original line range [start, end] (inclusive) - line numbers in the commit that introduced them
    pub orig_range: (u32, u32),
    /// Commit SHA that introduced this hunk
    pub commit_sha: String,
    /// Abbreviated commit SHA
    #[allow(dead_code)]
    pub abbrev_sha: String,
    /// Original author from Git blame
    pub original_author: String,
    /// Author email
    pub author_email: String,
    /// Author time (unix timestamp)
    pub author_time: i64,
    /// Author timezone (e.g. "+0000")
    pub author_tz: String,
    /// Committer name
    pub committer: String,
    /// Committer email
    pub committer_email: String,
    /// Committer time (unix timestamp)
    pub committer_time: i64,
    /// Committer timezone
    pub committer_tz: String,
    /// Whether this is a boundary commit
    pub is_boundary: bool,
}

#[derive(Debug, Clone)]
pub struct GitAiBlameOptions {
    // Line range options
    pub line_ranges: Vec<(u32, u32)>,

    pub newest_commit: Option<String>,

    // Output format options
    pub porcelain: bool,
    pub line_porcelain: bool,
    pub incremental: bool,
    pub show_name: bool,
    pub show_number: bool,
    pub show_email: bool,
    pub suppress_author: bool,
    pub show_stats: bool,

    // Commit display options
    pub long_rev: bool,
    pub raw_timestamp: bool,
    pub abbrev: Option<u32>,

    // Boundary options
    pub blank_boundary: bool,
    pub show_root: bool,

    // Movement detection options
    pub detect_moves: bool,
    pub detect_copies: u32, // Number of -C flags (0-3)
    pub move_threshold: Option<u32>,

    // Ignore options
    pub ignore_revs: Vec<String>,
    pub ignore_revs_file: Option<String>,

    // Color options
    pub color_lines: bool,
    pub color_by_age: bool,

    // Progress options
    pub progress: bool,

    // Date format
    pub date_format: Option<String>,

    // Content options
    pub contents_file: Option<String>,

    // Revision options
    #[allow(dead_code)]
    pub reverse: Option<String>,
    pub first_parent: bool,

    // Encoding
    pub encoding: Option<String>,

    // Use prompt hashes as name instead of author names
    pub use_prompt_hashes_as_names: bool,

    // Return all human authors as CheckpointKind::Human
    pub return_human_authors_as_human: bool,

    // No output
    pub no_output: bool,
}

impl Default for GitAiBlameOptions {
    fn default() -> Self {
        Self {
            line_ranges: Vec::new(),
            porcelain: false,
            newest_commit: None,
            line_porcelain: false,
            incremental: false,
            show_name: false,
            show_number: false,
            show_email: false,
            suppress_author: false,
            show_stats: false,
            long_rev: false,
            raw_timestamp: false,
            abbrev: None,
            blank_boundary: false,
            show_root: false,
            detect_moves: false,
            detect_copies: 0,
            move_threshold: None,
            ignore_revs: Vec::new(),
            ignore_revs_file: None,
            color_lines: false,
            color_by_age: false,
            progress: false,
            date_format: None,
            contents_file: None,
            reverse: None,
            first_parent: false,
            encoding: None,
            use_prompt_hashes_as_names: false,
            return_human_authors_as_human: false,
            no_output: false,
        }
    }
}

impl Repository {
    pub fn blame(
        &self,
        file_path: &str,
        options: &GitAiBlameOptions,
    ) -> Result<(HashMap<u32, String>, HashMap<String, PromptRecord>), GitAiError> {
        // Use repo root for file system operations
        let repo_root = self.workdir().or_else(|e| {
            Err(GitAiError::Generic(format!(
                "Repository has no working directory: {}",
                e
            )))
        })?;

        // Normalize the file path to be relative to repo root
        // This is important for AI authorship lookup which stores paths relative to repo root
        let file_path_buf = std::path::Path::new(file_path);
        let relative_file_path = if file_path_buf.is_absolute() {
            // Convert absolute path to relative path
            // Canonicalize both paths to handle symlinks (e.g., /var -> /private/var on macOS)
            let canonical_file_path = file_path_buf.canonicalize().map_err(|e| {
                GitAiError::Generic(format!(
                    "Failed to canonicalize file path '{}': {}",
                    file_path, e
                ))
            })?;
            let canonical_repo_root = repo_root.canonicalize().map_err(|e| {
                GitAiError::Generic(format!(
                    "Failed to canonicalize repository root '{}': {}",
                    repo_root.display(),
                    e
                ))
            })?;

            canonical_file_path
                .strip_prefix(&canonical_repo_root)
                .map_err(|_| {
                    GitAiError::Generic(format!(
                        "File path '{}' is not within repository root '{}'",
                        file_path,
                        repo_root.display()
                    ))
                })?
                .to_string_lossy()
                .to_string()
        } else {
            file_path.to_string()
        };

        let abs_file_path = repo_root.join(&relative_file_path);

        // Validate that the file exists
        if !abs_file_path.exists() {
            return Err(GitAiError::Generic(format!(
                "File not found: {}",
                abs_file_path.display()
            )));
        }

        // Read the current file content
        let file_content = fs::read_to_string(&abs_file_path)?;
        let lines: Vec<&str> = file_content.lines().collect();
        let total_lines = lines.len() as u32;

        // Determine the line ranges to process
        let line_ranges = if options.line_ranges.is_empty() {
            vec![(1, total_lines)]
        } else {
            options.line_ranges.clone()
        };

        // Validate line ranges
        for (start, end) in &line_ranges {
            if *start == 0 || *end == 0 || start > end || *end > total_lines {
                return Err(GitAiError::Generic(format!(
                    "Invalid line range: {}:{}. File has {} lines",
                    start, end, total_lines
                )));
            }
        }

        // Step 1: Get Git's native blame for all ranges
        let mut all_blame_hunks = Vec::new();
        for (start_line, end_line) in &line_ranges {
            let hunks = self.blame_hunks(&relative_file_path, *start_line, *end_line, options)?;
            all_blame_hunks.extend(hunks);
        }

        // Step 2: Overlay AI authorship information
        let (line_authors, prompt_records) =
            overlay_ai_authorship(self, &all_blame_hunks, &relative_file_path, options)?;

        if options.no_output {
            return Ok((line_authors, prompt_records));
        }

        // Output based on format
        if options.porcelain || options.line_porcelain {
            output_porcelain_format(
                self,
                &line_authors,
                &relative_file_path,
                &lines,
                &line_ranges,
                options,
            )?;
        } else if options.incremental {
            output_incremental_format(
                self,
                &line_authors,
                &relative_file_path,
                &lines,
                &line_ranges,
                options,
            )?;
        } else {
            output_default_format(
                self,
                &line_authors,
                &relative_file_path,
                &lines,
                &line_ranges,
                options,
            )?;
        }

        Ok((line_authors, prompt_records))
    }

    pub fn blame_hunks(
        &self,
        file_path: &str,
        start_line: u32,
        end_line: u32,
        options: &GitAiBlameOptions,
    ) -> Result<Vec<BlameHunk>, GitAiError> {
        // Build git blame --line-porcelain command
        let mut args = self.global_args_for_exec();
        args.push("blame".to_string());
        args.push("--line-porcelain".to_string());

        // Match previous behavior: ignore whitespace
        args.push("-w".to_string());

        // Respect ignore options in use
        for rev in &options.ignore_revs {
            args.push("--ignore-rev".to_string());
            args.push(rev.clone());
        }
        if let Some(file) = &options.ignore_revs_file {
            args.push("--ignore-revs-file".to_string());
            args.push(file.clone());
        }

        // Limit to specified range
        args.push("-L".to_string());
        args.push(format!("{},{}", start_line, end_line));

        // Support newest_commit option (equivalent to libgit2's newest_commit)
        // This limits blame to only consider commits up to and including the specified commit
        if let Some(ref commit) = options.newest_commit {
            args.push(commit.clone());
        }

        // Separator then file path
        args.push("--".to_string());
        args.push(file_path.to_string());

        let output = exec_git(&args)?;
        let stdout = String::from_utf8(output.stdout)?;

        // Parser state for current hunk
        #[derive(Default)]
        struct CurMeta {
            author: String,
            author_mail: String,
            author_time: i64,
            author_tz: String,
            committer: String,
            committer_mail: String,
            committer_time: i64,
            committer_tz: String,
            boundary: bool,
        }

        let mut hunks: Vec<BlameHunk> = Vec::new();
        let mut cur_commit: Option<String> = None;
        let mut cur_final_start: u32 = 0;
        let mut cur_orig_start: u32 = 0;
        let mut cur_group_size: u32 = 0;
        let mut cur_meta = CurMeta::default();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }

            if line.starts_with('\t') {
                // Content line; nothing to do, boundaries are driven by headers
                continue;
            }

            // Metadata lines
            if let Some(rest) = line.strip_prefix("author ") {
                cur_meta.author = rest.to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("author-mail ") {
                // Usually in form: <mail>
                cur_meta.author_mail = rest
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("author-time ") {
                if let Ok(t) = rest.trim().parse::<i64>() {
                    cur_meta.author_time = t;
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("author-tz ") {
                cur_meta.author_tz = rest.trim().to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("committer ") {
                cur_meta.committer = rest.to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("committer-mail ") {
                cur_meta.committer_mail = rest
                    .trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_string();
                continue;
            }
            if let Some(rest) = line.strip_prefix("committer-time ") {
                if let Ok(t) = rest.trim().parse::<i64>() {
                    cur_meta.committer_time = t;
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("committer-tz ") {
                cur_meta.committer_tz = rest.trim().to_string();
                continue;
            }
            if line == "boundary" {
                cur_meta.boundary = true;
                continue;
            }

            // Header line: either 4 fields (new hunk) or 3 fields (continuation)
            let mut parts = line.split_whitespace();
            let sha = parts.next().unwrap_or("");
            let p2 = parts.next().unwrap_or("");
            let p3 = parts.next().unwrap_or("");
            let p4 = parts.next();

            let is_header = !sha.is_empty()
                && sha.chars().all(|c| c.is_ascii_hexdigit())
                && !p2.is_empty()
                && !p3.is_empty();
            if !is_header {
                continue;
            }

            // If we encounter a new hunk header (4 fields), flush previous hunk first
            if p4.is_some() {
                if let Some(prev_sha) = cur_commit.take() {
                    // Push the previous hunk
                    let start = cur_final_start;
                    let end = if cur_group_size > 0 {
                        start + cur_group_size - 1
                    } else {
                        start
                    };
                    let orig_start = cur_orig_start;
                    let orig_end = if cur_group_size > 0 {
                        orig_start + cur_group_size - 1
                    } else {
                        orig_start
                    };

                    let abbrev_len = if options.long_rev {
                        40
                    } else {
                        options.abbrev.unwrap_or(7) as usize
                    };
                    let abbrev = if abbrev_len < prev_sha.len() {
                        prev_sha[..abbrev_len].to_string()
                    } else {
                        prev_sha.clone()
                    };

                    hunks.push(BlameHunk {
                        range: (start, end),
                        orig_range: (orig_start, orig_end),
                        commit_sha: prev_sha,
                        abbrev_sha: abbrev,
                        original_author: cur_meta.author.clone(),
                        author_email: cur_meta.author_mail.clone(),
                        author_time: cur_meta.author_time,
                        author_tz: cur_meta.author_tz.clone(),
                        committer: cur_meta.committer.clone(),
                        committer_email: cur_meta.committer_mail.clone(),
                        committer_time: cur_meta.committer_time,
                        committer_tz: cur_meta.committer_tz.clone(),
                        is_boundary: cur_meta.boundary,
                    });
                }

                // Start new hunk
                cur_commit = Some(sha.to_string());
                // According to docs: fields are orig_lineno, final_lineno, group_size
                let orig_start = p2.parse::<u32>().unwrap_or(0);
                let final_start = p3.parse::<u32>().unwrap_or(0);
                let group = p4.unwrap_or("1").parse::<u32>().unwrap_or(1);
                cur_orig_start = orig_start;
                cur_final_start = final_start;
                cur_group_size = group;
                // Reset metadata for the new hunk
                cur_meta = CurMeta::default();
            } else {
                // 3-field header: continuation line within current hunk
                // Nothing to do for grouping since we use recorded group_size
                // Metadata remains from the first line of the hunk
                if cur_commit.is_none() {
                    // Defensive: if no current hunk, start one with size 1
                    cur_commit = Some(sha.to_string());
                    cur_orig_start = p2.parse::<u32>().unwrap_or(0);
                    cur_final_start = p3.parse::<u32>().unwrap_or(0);
                    cur_group_size = 1;
                }
            }
        }

        // Flush the final hunk if present
        if let Some(prev_sha) = cur_commit.take() {
            let start = cur_final_start;
            let end = if cur_group_size > 0 {
                start + cur_group_size - 1
            } else {
                start
            };
            let orig_start = cur_orig_start;
            let orig_end = if cur_group_size > 0 {
                orig_start + cur_group_size - 1
            } else {
                orig_start
            };

            let abbrev_len = if options.long_rev {
                40
            } else {
                options.abbrev.unwrap_or(7) as usize
            };
            let abbrev = if abbrev_len < prev_sha.len() {
                prev_sha[..abbrev_len].to_string()
            } else {
                prev_sha.clone()
            };

            hunks.push(BlameHunk {
                range: (start, end),
                orig_range: (orig_start, orig_end),
                commit_sha: prev_sha,
                abbrev_sha: abbrev,
                original_author: cur_meta.author.clone(),
                author_email: cur_meta.author_mail.clone(),
                author_time: cur_meta.author_time,
                author_tz: cur_meta.author_tz.clone(),
                committer: cur_meta.committer.clone(),
                committer_email: cur_meta.committer_mail.clone(),
                committer_time: cur_meta.committer_time,
                committer_tz: cur_meta.committer_tz.clone(),
                is_boundary: cur_meta.boundary,
            });
        }

        Ok(hunks)
    }
}

fn overlay_ai_authorship(
    repo: &Repository,
    blame_hunks: &[BlameHunk],
    file_path: &str,
    options: &GitAiBlameOptions,
) -> Result<(HashMap<u32, String>, HashMap<String, PromptRecord>), GitAiError> {
    let mut line_authors: HashMap<u32, String> = HashMap::new();
    let mut prompt_records: HashMap<String, PromptRecord> = HashMap::new();

    // Group hunks by commit SHA to avoid repeated lookups
    let mut commit_authorship_cache: HashMap<String, Option<AuthorshipLog>> = HashMap::new();
    // Cache for foreign prompts to avoid repeated grepping
    let mut foreign_prompts_cache: HashMap<String, Option<PromptRecord>> = HashMap::new();

    for hunk in blame_hunks {
        // Check if we've already looked up this commit's authorship
        let authorship_log = if let Some(cached) = commit_authorship_cache.get(&hunk.commit_sha) {
            cached.clone()
        } else {
            // Try to get authorship log for this commit
            let authorship = match get_reference_as_authorship_log_v3(repo, &hunk.commit_sha) {
                Ok(v3_log) => Some(v3_log),
                Err(_) => None, // No AI authorship data for this commit
            };
            commit_authorship_cache.insert(hunk.commit_sha.clone(), authorship.clone());
            authorship
        };

        // If we have AI authorship data, look up the author for lines in this hunk
        if let Some(authorship_log) = authorship_log {
            // Check each line in this hunk for AI authorship using compact schema
            // IMPORTANT: Use the original line numbers from the commit, not the current line numbers
            let num_lines = hunk.range.1 - hunk.range.0 + 1;
            for i in 0..num_lines {
                let current_line_num = hunk.range.0 + i;
                let orig_line_num = hunk.orig_range.0 + i;

                if let Some((author, prompt_hash, prompt)) = authorship_log.get_line_attribution(
                    repo,
                    file_path,
                    orig_line_num,
                    &mut foreign_prompts_cache,
                ) {
                    // If this line is AI-assisted, display the tool name; otherwise the human username
                    if let Some(prompt_record) = prompt {
                        let prompt_hash = prompt_hash.unwrap();
                        if options.use_prompt_hashes_as_names {
                            line_authors.insert(current_line_num, prompt_hash.clone());
                        } else {
                            line_authors
                                .insert(current_line_num, prompt_record.agent_id.tool.clone());
                        }
                        prompt_records.insert(prompt_hash, prompt_record.clone());
                    } else {
                        if options.return_human_authors_as_human {
                            line_authors.insert(
                                current_line_num,
                                CheckpointKind::Human.to_str().to_string(),
                            );
                        } else {
                            line_authors.insert(current_line_num, author.username.clone());
                        }
                    }
                } else {
                    // Fall back to original author if no AI authorship
                    if options.return_human_authors_as_human {
                        line_authors
                            .insert(current_line_num, CheckpointKind::Human.to_str().to_string());
                    } else {
                        line_authors.insert(current_line_num, hunk.original_author.clone());
                    }
                }
            }
        } else {
            // No authorship log, use original author for all lines in hunk
            for line_num in hunk.range.0..=hunk.range.1 {
                if options.return_human_authors_as_human {
                    line_authors.insert(line_num, CheckpointKind::Human.to_str().to_string());
                } else {
                    line_authors.insert(line_num, hunk.original_author.clone());
                }
            }
        }
    }

    Ok((line_authors, prompt_records))
}

#[allow(unused_variables)]
#[allow(dead_code)]
fn print_blame_summary(line_authors: &HashMap<u32, String>, start_line: u32, end_line: u32) {
    println!("{}", "=".repeat(80));

    let mut author_stats: HashMap<String, u32> = HashMap::new();
    let mut total_lines = 0;

    for line_num in start_line..=end_line {
        let author = line_authors
            .get(&line_num)
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        *author_stats.entry(author.to_string()).or_insert(0) += 1;
        total_lines += 1;
    }

    // Find the longest author name for column width
    let max_author_len = author_stats
        .keys()
        .map(|name| name.len())
        .max()
        .unwrap_or(0);

    // Sort authors by line count (descending)
    let mut sorted_authors: Vec<_> = author_stats.iter().collect();
    sorted_authors.sort_by(|a, b| b.1.cmp(a.1));

    for (author, count) in sorted_authors {
        let percentage = (*count as f64 / total_lines as f64) * 100.0;
        println!(
            "{:>width$} {:>5} {:>8.1}%",
            author,
            count,
            percentage,
            width = max_author_len
        );
    }
}

fn output_porcelain_format(
    repo: &Repository,
    _line_authors: &HashMap<u32, String>,
    file_path: &str,
    lines: &[&str],
    line_ranges: &[(u32, u32)],
    options: &GitAiBlameOptions,
) -> Result<(), GitAiError> {
    // Build a map from line number to BlameHunk for fast lookup
    let mut line_to_hunk: HashMap<u32, BlameHunk> = HashMap::new();
    for (start_line, end_line) in line_ranges {
        let h = repo.blame_hunks(file_path, *start_line, *end_line, options)?;
        for hunk in h {
            for line_num in hunk.range.0..=hunk.range.1 {
                line_to_hunk.insert(line_num, hunk.clone());
            }
        }
    }

    let mut last_hunk_id = None;
    for (start_line, end_line) in line_ranges {
        for line_num in *start_line..=*end_line {
            let line_index = (line_num - 1) as usize;
            let line_content = if line_index < lines.len() {
                lines[line_index]
            } else {
                ""
            };

            if let Some(hunk) = line_to_hunk.get(&line_num) {
                let author_name = &hunk.original_author;
                let commit_sha = &hunk.commit_sha;
                let author_email = &hunk.author_email;
                let author_time = hunk.author_time;
                let author_tz = &hunk.author_tz;
                let committer_name = &hunk.committer;
                let committer_email = &hunk.committer_email;
                let committer_time = hunk.committer_time;
                let committer_tz = &hunk.committer_tz;
                let boundary = hunk.is_boundary;
                let filename = file_path;

                // Retrieve the commit summary directly from the commit object
                let commit = repo.find_commit(commit_sha.clone())?;
                let summary = commit.summary()?;

                let hunk_id = (commit_sha.clone(), hunk.range.0);
                if options.line_porcelain {
                    if last_hunk_id.as_ref() != Some(&hunk_id) {
                        // First line of hunk: 4-field header
                        println!(
                            "{} {} {} {}",
                            commit_sha,
                            line_num,
                            line_num,
                            hunk.range.1 - hunk.range.0 + 1
                        );
                        last_hunk_id = Some(hunk_id);
                    } else {
                        // Subsequent lines: 3-field header
                        println!("{} {} {}", commit_sha, line_num, line_num);
                    }
                    println!("author {}", author_name);
                    println!("author-mail <{}>", author_email);
                    println!("author-time {}", author_time);
                    println!("author-tz {}", author_tz);
                    println!("committer {}", committer_name);
                    println!("committer-mail <{}>", committer_email);
                    println!("committer-time {}", committer_time);
                    println!("committer-tz {}", committer_tz);
                    println!("summary {}", summary);
                    if boundary {
                        println!("boundary");
                    }
                    println!("filename {}", filename);
                    println!("\t{}", line_content);
                } else if options.porcelain {
                    if last_hunk_id.as_ref() != Some(&hunk_id) {
                        // Print full block for first line of hunk
                        println!(
                            "{} {} {} {}",
                            commit_sha,
                            line_num,
                            line_num,
                            hunk.range.1 - hunk.range.0 + 1
                        );
                        println!("author {}", author_name);
                        println!("author-mail <{}>", author_email);
                        println!("author-time {}", author_time);
                        println!("author-tz {}", author_tz);
                        println!("committer {}", committer_name);
                        println!("committer-mail <{}>", committer_email);
                        println!("committer-time {}", committer_time);
                        println!("committer-tz {}", committer_tz);
                        println!("summary {}", summary);
                        if boundary {
                            println!("boundary");
                        }
                        println!("filename {}", filename);
                        println!("\t{}", line_content);
                        last_hunk_id = Some(hunk_id);
                    } else {
                        // For subsequent lines, print only the header and content (no metadata block)
                        println!("{} {} {}", commit_sha, line_num, line_num);
                        println!("\t{}", line_content);
                    }
                }
            }
        }
    }
    Ok(())
}

fn output_incremental_format(
    repo: &Repository,
    _line_authors: &HashMap<u32, String>,
    file_path: &str,
    _lines: &[&str],
    line_ranges: &[(u32, u32)],
    options: &GitAiBlameOptions,
) -> Result<(), GitAiError> {
    // Build a map from line number to BlameHunk for fast lookup
    let mut line_to_hunk: HashMap<u32, BlameHunk> = HashMap::new();
    for (start_line, end_line) in line_ranges {
        let h = repo.blame_hunks(file_path, *start_line, *end_line, options)?;
        for hunk in h {
            for line_num in hunk.range.0..=hunk.range.1 {
                line_to_hunk.insert(line_num, hunk.clone());
            }
        }
    }

    let mut last_hunk_id = None;
    for (start_line, end_line) in line_ranges {
        for line_num in *start_line..=*end_line {
            if let Some(hunk) = line_to_hunk.get(&line_num) {
                // For incremental format, use the original git author, not AI authorship
                let author_name = &hunk.original_author;
                let commit_sha = &hunk.commit_sha;
                let author_email = &hunk.author_email;
                let author_time = hunk.author_time;
                let author_tz = &hunk.author_tz;
                let committer_name = &hunk.committer;
                let committer_email = &hunk.committer_email;
                let committer_time = hunk.committer_time;
                let committer_tz = &hunk.committer_tz;

                // Only print the full block for the first line of a hunk
                let hunk_id = (hunk.commit_sha.clone(), hunk.range.0);
                if last_hunk_id.as_ref() != Some(&hunk_id) {
                    // Print full block - match git's format exactly
                    println!(
                        "{} {} {} {}",
                        commit_sha,
                        line_num,
                        line_num,
                        hunk.range.1 - hunk.range.0 + 1
                    );
                    println!("author {}", author_name);
                    println!("author-mail <{}>", author_email);
                    println!("author-time {}", author_time);
                    println!("author-tz {}", author_tz);
                    println!("committer {}", committer_name);
                    println!("committer-mail <{}>", committer_email);
                    println!("committer-time {}", committer_time);
                    println!("committer-tz {}", committer_tz);
                    println!("summary Initial commit");
                    if hunk.is_boundary {
                        println!("boundary");
                    }
                    println!("filename {}", file_path);
                    last_hunk_id = Some(hunk_id);
                }
                // For incremental, no content lines (no \tLine)
            } else {
                // Fallback for lines without blame info
                println!(
                    "0000000000000000000000000000000000000000 {} {} 1",
                    line_num, line_num
                );
                println!("author unknown");
                println!("author-mail <unknown@example.com>");
                println!("author-time 0");
                println!("author-tz +0000");
                println!("committer unknown");
                println!("committer-mail <unknown@example.com>");
                println!("committer-time 0");
                println!("committer-tz +0000");
                println!("summary unknown");
                println!("filename {}", file_path);
            }
        }
    }
    Ok(())
}

fn output_default_format(
    repo: &Repository,
    line_authors: &HashMap<u32, String>,
    file_path: &str,
    lines: &[&str],
    line_ranges: &[(u32, u32)],
    options: &GitAiBlameOptions,
) -> Result<(), GitAiError> {
    let mut output = String::new();

    // Build a map from line number to BlameHunk for fast lookup
    let mut line_to_hunk: HashMap<u32, BlameHunk> = HashMap::new();
    for (start_line, end_line) in line_ranges {
        let h = repo.blame_hunks(file_path, *start_line, *end_line, options)?;
        for hunk in h {
            for line_num in hunk.range.0..=hunk.range.1 {
                line_to_hunk.insert(line_num, hunk.clone());
            }
        }
    }

    for (start_line, end_line) in line_ranges {
        for line_num in *start_line..=*end_line {
            let line_index = (line_num - 1) as usize;
            let line_content = if line_index < lines.len() {
                lines[line_index]
            } else {
                ""
            };

            if let Some(hunk) = line_to_hunk.get(&line_num) {
                // Determine hash length - match git blame default (7 chars)
                let hash_len = if options.long_rev {
                    40 // Full hash for long revision
                } else if let Some(abbrev) = options.abbrev {
                    abbrev as usize
                } else {
                    7 // Default 7 chars
                };
                let sha = if hash_len < hunk.commit_sha.len() {
                    &hunk.commit_sha[..hash_len]
                } else {
                    &hunk.commit_sha
                };

                // Add boundary marker if this is a boundary commit
                let boundary_marker = if hunk.is_boundary && options.blank_boundary {
                    "^"
                } else {
                    ""
                };
                let full_sha = if hunk.is_boundary && options.blank_boundary {
                    format!("{}{}", boundary_marker, "        ") // Empty hash for boundary
                } else {
                    format!("{}{}", boundary_marker, sha)
                };

                // Get the author for this line (AI authorship or original)
                let author = line_authors.get(&line_num).unwrap_or(&hunk.original_author);

                // Format date according to options
                let date_str = format_blame_date(hunk.author_time, &hunk.author_tz, options);

                // Handle different output formats based on flags
                let author_display = if options.suppress_author {
                    "".to_string()
                } else if options.show_email {
                    format!("{} <{}>", author, &hunk.author_email)
                } else {
                    author.to_string()
                };

                let _filename_display = if options.show_name {
                    format!("{} ", file_path)
                } else {
                    "".to_string()
                };

                let _number_display = if options.show_number {
                    format!("{} ", line_num)
                } else {
                    "".to_string()
                };

                // Format exactly like git blame: sha (author date line) code
                if options.suppress_author {
                    // Suppress author format: sha line_number) code
                    output.push_str(&format!("{} {}) {}\n", full_sha, line_num, line_content));
                } else {
                    // Normal format: sha (author date line) code
                    if options.show_name {
                        // Show filename format: sha filename (author date line) code
                        output.push_str(&format!(
                            "{} {} ({} {} {:>4}) {}\n",
                            full_sha, file_path, author_display, date_str, line_num, line_content
                        ));
                    } else if options.show_number {
                        // Show number format: sha line_number (author date line) code (matches git's -n output)
                        output.push_str(&format!(
                            "{} {} ({} {} {:>4}) {}\n",
                            full_sha, line_num, author_display, date_str, line_num, line_content
                        ));
                    } else {
                        // Normal format: sha (author date line) code
                        output.push_str(&format!(
                            "{} ({} {} {:>4}) {}\n",
                            full_sha, author_display, date_str, line_num, line_content
                        ));
                    }
                }
            } else {
                // Fallback for lines without blame info
                output.push_str(&format!(
                    "{:<8} (unknown        1970-01-01 00:00:00 +0000    {:>4}) {}\n",
                    "????????", line_num, line_content
                ));
            }
        }
    }

    // Print stats if requested (at the end, like git blame)
    if options.show_stats {
        // Append git-like stats lines to output string
        let stats = "num read blob: 1\nnum get patch: 0\nnum commits: 0\n";
        output.push_str(stats);
    }

    // Output handling - respect pager environment variables
    let pager = std::env::var("GIT_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_else(|_| "less".to_string());

    // If pager is set to "cat" or empty, output directly
    if pager == "cat" || pager.is_empty() {
        print!("{}", output);
    } else if io::stdout().is_terminal() {
        // Try to use the specified pager
        match std::process::Command::new(&pager)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.as_mut() {
                    if stdin.write_all(output.as_bytes()).is_ok() {
                        let _ = child.wait();
                    } else {
                        // Fall back to direct output if pager fails
                        print!("{}", output);
                    }
                } else {
                    // Fall back to direct output if pager fails
                    print!("{}", output);
                }
            }
            Err(_) => {
                // Fall back to direct output if pager fails
                print!("{}", output);
            }
        }
    } else {
        // Not a terminal, output directly
        print!("{}", output);
    }
    Ok(())
}

fn format_blame_date(author_time: i64, author_tz: &str, options: &GitAiBlameOptions) -> String {
    let dt = DateTime::from_timestamp(author_time, 0)
        .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());

    // Parse timezone string like +0200 or -0500
    let offset = if author_tz.len() == 5 {
        let sign = if &author_tz[0..1] == "+" { 1 } else { -1 };
        let hours: i32 = author_tz[1..3].parse().unwrap_or(0);
        let mins: i32 = author_tz[3..5].parse().unwrap_or(0);
        FixedOffset::east_opt(sign * (hours * 3600 + mins * 60))
            .unwrap_or(FixedOffset::east_opt(0).unwrap())
    } else {
        FixedOffset::east_opt(0).unwrap()
    };

    let dt = offset.from_utc_datetime(&dt.naive_utc());

    // Format date according to options (default: iso)
    if let Some(fmt) = &options.date_format {
        // TODO: support all git date formats
        match fmt.as_str() {
            "iso" | "iso8601" => dt.format("%Y-%m-%d %H:%M:%S %z").to_string(),
            "short" => dt.format("%Y-%m-%d").to_string(),
            "relative" => format!("{} seconds ago", (Utc::now().timestamp() - author_time)),
            _ => dt.format("%Y-%m-%d %H:%M:%S %z").to_string(),
        }
    } else {
        dt.format("%Y-%m-%d %H:%M:%S %z").to_string()
    }
}

pub fn parse_blame_args(args: &[String]) -> Result<(String, GitAiBlameOptions), GitAiError> {
    let mut options = GitAiBlameOptions::default();
    let mut file_path = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            // Line range options
            "-L" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic("Missing argument for -L".to_string()));
                }
                let range_str = &args[i + 1];
                if let Some((start, end)) = parse_line_range(range_str) {
                    options.line_ranges.push((start, end));
                } else {
                    return Err(GitAiError::Generic(format!(
                        "Invalid line range: {}",
                        range_str
                    )));
                }
                i += 2;
            }

            // Output format options
            "--porcelain" => {
                options.porcelain = true;
                i += 1;
            }
            "--line-porcelain" => {
                options.line_porcelain = true;
                options.porcelain = true; // Implies --porcelain
                i += 1;
            }
            "--incremental" => {
                options.incremental = true;
                i += 1;
            }
            "-f" | "--show-name" => {
                options.show_name = true;
                i += 1;
            }
            "-n" | "--show-number" => {
                options.show_number = true;
                i += 1;
            }
            "-e" | "--show-email" => {
                options.show_email = true;
                i += 1;
            }
            "-s" => {
                options.suppress_author = true;
                i += 1;
            }
            "--show-stats" => {
                options.show_stats = true;
                i += 1;
            }

            // Commit display options
            "-l" => {
                options.long_rev = true;
                i += 1;
            }
            "-t" => {
                options.raw_timestamp = true;
                i += 1;
            }
            "--abbrev" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --abbrev".to_string(),
                    ));
                }
                if let Ok(n) = args[i + 1].parse::<u32>() {
                    options.abbrev = Some(n);
                } else {
                    return Err(GitAiError::Generic(
                        "Invalid number for --abbrev".to_string(),
                    ));
                }
                i += 2;
            }

            // Boundary options
            "-b" => {
                options.blank_boundary = true;
                i += 1;
            }
            "--root" => {
                options.show_root = true;
                i += 1;
            }

            // Movement detection options
            "-M" => {
                options.detect_moves = true;
                if i + 1 < args.len() {
                    if let Ok(threshold) = args[i + 1].parse::<u32>() {
                        options.move_threshold = Some(threshold);
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            "-C" => {
                options.detect_copies = (options.detect_copies + 1).min(3);
                if i + 1 < args.len() {
                    if let Ok(threshold) = args[i + 1].parse::<u32>() {
                        options.move_threshold = Some(threshold);
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }

            // Ignore options
            "--ignore-rev" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --ignore-rev".to_string(),
                    ));
                }
                options.ignore_revs.push(args[i + 1].clone());
                i += 2;
            }
            "--ignore-revs-file" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --ignore-revs-file".to_string(),
                    ));
                }
                options.ignore_revs_file = Some(args[i + 1].clone());
                i += 2;
            }

            // Color options
            "--color-lines" => {
                options.color_lines = true;
                i += 1;
            }
            "--color-by-age" => {
                options.color_by_age = true;
                i += 1;
            }

            // Progress options
            "--progress" => {
                options.progress = true;
                i += 1;
            }

            // Date format
            "--date" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --date".to_string(),
                    ));
                }
                options.date_format = Some(args[i + 1].clone());
                i += 2;
            }

            // Content options
            "--contents" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --contents".to_string(),
                    ));
                }
                options.contents_file = Some(args[i + 1].clone());
                i += 2;
            }

            // Revision options
            "--reverse" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --reverse".to_string(),
                    ));
                }
                options.reverse = Some(args[i + 1].clone());
                i += 2;
            }
            "--first-parent" => {
                options.first_parent = true;
                i += 1;
            }

            // Encoding
            "--encoding" => {
                if i + 1 >= args.len() {
                    return Err(GitAiError::Generic(
                        "Missing argument for --encoding".to_string(),
                    ));
                }
                options.encoding = Some(args[i + 1].clone());
                i += 2;
            }

            // File path (non-option argument)
            arg if !arg.starts_with('-') => {
                if file_path.is_none() {
                    file_path = Some(arg.to_string());
                } else {
                    return Err(GitAiError::Generic(
                        "Multiple file paths specified".to_string(),
                    ));
                }
                i += 1;
            }

            // Unknown option
            _ => {
                return Err(GitAiError::Generic(format!("Unknown option: {}", args[i])));
            }
        }
    }

    let file_path =
        file_path.ok_or_else(|| GitAiError::Generic("No file path specified".to_string()))?;

    Ok((file_path, options))
}

fn parse_line_range(range_str: &str) -> Option<(u32, u32)> {
    if let Some(dash_pos) = range_str.find(',') {
        let start_str = &range_str[..dash_pos];
        let end_str = &range_str[dash_pos + 1..];

        if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
            return Some((start, end));
        }
    } else if let Ok(line) = range_str.parse::<u32>() {
        return Some((line, line));
    }

    None
}

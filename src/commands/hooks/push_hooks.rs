use crate::git::cli_parser::{ParsedGitInvocation, is_dry_run};
use crate::git::repository::Repository;
use crate::git::sync_authorship::push_authorship_notes;
use crate::utils::debug_log;

pub fn push_post_command_hook(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
    exit_status: std::process::ExitStatus,
) {
    if is_dry_run(&parsed_args.command_args)
        || !exit_status.success()
        || parsed_args
            .command_args
            .iter()
            .any(|a| a == "-d" || a == "--delete")
        || parsed_args.command_args.iter().any(|a| a == "--mirror")
    {
        return;
    }

    let remotes = repository.remotes().ok();
    let remote_names: Vec<String> = remotes
        .as_ref()
        .map(|r| {
            (0..r.len())
                .filter_map(|i| r.get(i).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 2) Push authorship refs to the appropriate remote
    let positional_remote = extract_remote_from_push_args(&parsed_args.command_args, &remote_names);

    let specified_remote = positional_remote.or_else(|| {
        parsed_args
            .command_args
            .iter()
            .find(|a| remote_names.iter().any(|r| r == *a))
            .cloned()
    });

    let remote = specified_remote
        .or_else(|| repository.upstream_remote().ok().flatten())
        .or_else(|| repository.get_default_remote().ok().flatten());

    if let Some(remote) = remote {
        _ = push_authorship_notes(repository, remote.as_str());
    } else {
        // No remotes configured; skip silently
        debug_log("no remotes found for authorship push; skipping");
    }
}

fn extract_remote_from_push_args(args: &[String], known_remotes: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return args.get(i + 1).cloned();
        }
        if arg.starts_with('-') {
            if let Some((flag, value)) = is_push_option_with_inline_value(arg) {
                if flag == "--repo" {
                    return Some(value.to_string());
                }
                i += 1;
                continue;
            }

            if option_consumes_separate_value(arg.as_str()) {
                if arg == "--repo" {
                    return args.get(i + 1).cloned();
                }
                i += 2;
                continue;
            }

            i += 1;
            continue;
        }
        return Some(arg.clone());
    }

    known_remotes
        .iter()
        .find(|r| args.iter().any(|arg| arg == *r))
        .cloned()
}

fn is_push_option_with_inline_value(arg: &str) -> Option<(&str, &str)> {
    if let Some((flag, value)) = arg.split_once('=') {
        Some((flag, value))
    } else if (arg.starts_with("-C") || arg.starts_with("-c")) && arg.len() > 2 {
        // Treat -C<path> or -c<name>=<value> as inline values
        let flag = &arg[..2];
        let value = &arg[2..];
        Some((flag, value))
    } else {
        None
    }
}

fn option_consumes_separate_value(arg: &str) -> bool {
    matches!(
        arg,
        "--repo" | "--receive-pack" | "--exec" | "-o" | "--push-option" | "-c" | "-C"
    )
}

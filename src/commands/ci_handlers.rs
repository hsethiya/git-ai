use crate::ci::github::{get_github_ci_context, install_github_ci_workflow};
use crate::ci::ci_context::{CiContext, CiEvent};
use crate::git::repository::find_repository_in_path;
use crate::utils::debug_log;

pub fn handle_ci(args: &[String]) {
    if args.is_empty() {
        print_ci_help_and_exit();
    }

    match args[0].as_str() {
        "github" => {
            handle_ci_github(&args[1..]);
        }
        "local" => {
            handle_ci_local(&args[1..]);
        }
        _ => {
            eprintln!("Unknown ci subcommand: {}", args[0]);
            print_ci_help_and_exit();
        }
    }
}

fn handle_ci_github(args: &[String]) {
    if args.is_empty() {
        print_ci_github_help_and_exit();
    }
    // Subcommands: install | (default: run in CI context)
    match args[0].as_str() {
        "run" => {
            let ci_context = get_github_ci_context();
            match ci_context {
                Ok(Some(ci_context)) => {
                    debug_log(&format!("GitHub CI context: {:?}", ci_context));
                    if let Err(e) = ci_context.run() {
                        eprintln!("Error running GitHub CI context: {}", e);
                        std::process::exit(1);
                    }
                    if let Err(e) = ci_context.teardown() {
                        eprintln!("Error tearing down GitHub CI context: {}", e);
                        std::process::exit(1);
                    }
                    debug_log("GitHub CI context teared down");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to get GitHub CI context: {}", e);
                    std::process::exit(1);
                }
                Ok(None) => {
                    eprintln!("No GitHub CI context found");
                    std::process::exit(1);
                }
            }
        }
        "install" => {
            match install_github_ci_workflow() {
                Ok(path) => {
                    println!(
                        "Installed GitHub Actions workflow to {}",
                        path.display()
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Failed to install GitHub CI workflow: {}", e);
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("Unknown ci github subcommand: {}", other);
            print_ci_help_and_exit();
        }
    }
}

fn handle_ci_local(args: &[String]) {
    if args.is_empty() {
        print_ci_local_help_and_exit();
    }

    let event = args[0].as_str();
    let event_args: &[String] = &args[1..];

    // Simple flag parser over remaining args: --key value
    let flag = |name: &str| -> Option<String> {
        let mut i = 0usize;
        while i < event_args.len() {
            if event_args[i] == name {
                if i + 1 < event_args.len() {
                    return Some(event_args[i + 1].clone());
                } else {
                    eprintln!("Missing value for flag {}", name);
                    std::process::exit(1);
                }
            }
            i += 1;
        }
        None
    };

    // Open current repo
    let repo = match find_repository_in_path(".") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open repository in current directory: {}", e);
            std::process::exit(1);
        }
    };

    match event {
        "merge" => {
            // Required inputs for merge
            let merge_commit_sha = match flag("--merge-commit-sha") {
                Some(v) => v,
                None => {
                    eprintln!("--merge-commit-sha is required");
                    std::process::exit(1);
                }
            };

            let base_ref = match flag("--base-ref") {
                Some(v) => v,
                None => {
                    eprintln!("--base-ref is required (e.g., main)");
                    std::process::exit(1);
                }
            };

            // All flags required for merge
            let head_ref = match flag("--head-ref") {
                Some(v) => v,
                None => {
                    eprintln!("--head-ref is required");
                    std::process::exit(1);
                }
            };

            let head_sha = match flag("--head-sha") {
                Some(v) => v,
                None => {
                    eprintln!("--head-sha is required");
                    std::process::exit(1);
                }
            };

            let base_sha = match flag("--base-sha") {
                Some(v) => v,
                None => {
                    eprintln!("--base-sha is required");
                    std::process::exit(1);
                }
            };

            let ctx = CiContext {
                repo,
                event: CiEvent::Merge {
                    merge_commit_sha,
                    head_ref,
                    head_sha,
                    base_ref,
                    base_sha,
                },
                // Not used for local runs; teardown not invoked
                temp_dir: std::path::PathBuf::from("."),
            };

            debug_log(&format!("Local CI context: {:?}", ctx));
            if let Err(e) = ctx.run() {
                eprintln!("Error running local CI: {}", e);
                std::process::exit(1);
            }

            println!("Local CI (merge) completed successfully");
            std::process::exit(0);
        }
        other => {
            eprintln!("Unknown local CI event: {}", other);
            print_ci_local_help_and_exit();
        }
    }
}

fn print_ci_help_and_exit() -> ! {
    eprintln!("git-ai ci - Continuous integration utilities");
    eprintln!("");
    eprintln!("Usage: git-ai ci <subcommand> [args...]");
    eprintln!("");
    eprintln!("Subcommands:");
    eprintln!("  github           GitHub CI");
    eprintln!("    run            Run GitHub CI in current repo");
    eprintln!("    install        Install/update workflow in current repo");
    eprintln!("  local            Run CI locally by event name and flags");
    eprintln!("                   Usage: git-ai ci local <event> [flags]");
    eprintln!("                   Events:");
    eprintln!("                     merge  --merge-commit-sha <sha> --base-ref <ref> --head-ref <ref> --head-sha <sha> --base-sha <sha>");
    std::process::exit(1);
}

fn print_ci_local_help_and_exit() -> ! {
    eprintln!("git-ai ci local - Run CI locally by event name and flags");
    eprintln!("");
    eprintln!("Usage: git-ai ci local <event> [flags]");
    eprintln!("");
    eprintln!("Events:");
    eprintln!("  merge  --merge-commit-sha <sha> --base-ref <ref> --head-ref <ref> --head-sha <sha> --base-sha <sha>");
    std::process::exit(1);
}

fn print_ci_github_help_and_exit() -> ! {
    eprintln!("git-ai ci github - GitHub CI utilities");
    eprintln!("");
    eprintln!("Usage: git-ai ci github <subcommand> [args...]");
    eprintln!("");
    eprintln!("Subcommands:");
    eprintln!("  run            Run GitHub CI in current repo");
    eprintln!("  install        Install/update workflow in current repo");
    std::process::exit(1);
}

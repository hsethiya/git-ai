mod commands;
mod config;
mod error;
mod git;
mod log_fmt;
mod utils;

use clap::Parser;
use git_ai::utils::debug_log;

#[derive(Parser)]
#[command(name = "git-ai")]
#[command(about = "git proxy with AI authorship tracking", long_about = None)]
#[command(disable_help_flag = true, disable_version_flag = true)]
struct Cli {
    /// Git command and arguments
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

fn main() {
    // Get the binary name that was called
    let binary_name = std::env::args_os()
        .next()
        .and_then(|arg| arg.into_string().ok())
        .and_then(|path| {
            std::path::Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or("git-ai".to_string());

    debug_log(&format!("calling {:?}", &binary_name));

    let cli = Cli::parse();
    if binary_name == "git-ai" {
        commands::git_ai_handlers::handle_git_ai(&cli.args);
        std::process::exit(0);
    }

    commands::git_handlers::handle_git(&cli.args);
}

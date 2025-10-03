use std::fmt;

#[derive(Debug)]
pub enum GitAiError {
    GitError(git2::Error),
    IoError(std::io::Error),
    /// Errors from invoking the git CLI that exited with a non-zero status
    GitCliError {
        code: Option<i32>,
        stderr: String,
        args: Vec<String>,
    },
    JsonError(serde_json::Error),
    Utf8Error(std::str::Utf8Error),
    FromUtf8Error(std::string::FromUtf8Error),
    PresetError(String),
    Generic(String),
}

impl fmt::Display for GitAiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitAiError::GitError(e) => write!(f, "Git error: {}", e),
            GitAiError::IoError(e) => write!(f, "IO error: {}", e),
            GitAiError::GitCliError { code, stderr, args } => match code {
                Some(c) => write!(f, "Git CLI ({}) failed with exit code {}: {}", args.join(" "), c, stderr),
                None => write!(f, "Git CLI ({}) failed: {}", args.join(" "), stderr),
            },
            GitAiError::JsonError(e) => write!(f, "JSON error: {}", e),
            GitAiError::Utf8Error(e) => write!(f, "UTF-8 error: {}", e),
            GitAiError::FromUtf8Error(e) => write!(f, "From UTF-8 error: {}", e),
            GitAiError::PresetError(e) => write!(f, "{}", e),
            GitAiError::Generic(e) => write!(f, "Generic error: {}", e),
        }
    }
}

impl std::error::Error for GitAiError {}

impl From<git2::Error> for GitAiError {
    fn from(err: git2::Error) -> Self {
        GitAiError::GitError(err)
    }
}

impl From<std::io::Error> for GitAiError {
    fn from(err: std::io::Error) -> Self {
        GitAiError::IoError(err)
    }
}

impl From<serde_json::Error> for GitAiError {
    fn from(err: serde_json::Error) -> Self {
        GitAiError::JsonError(err)
    }
}

impl From<std::str::Utf8Error> for GitAiError {
    fn from(err: std::str::Utf8Error) -> Self {
        GitAiError::Utf8Error(err)
    }
}

impl From<std::string::FromUtf8Error> for GitAiError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        GitAiError::FromUtf8Error(err)
    }
}

//! The checks that run when a worker's turn ends.

use crate::rpc::RpcError;
use crate::task::checks::{CHECKS_PATH, ChecksFile, asked_checks, merged, parse_checks};
use crate::task::git;
use crate::task::types::Asked;

/// The checks a task runs: the file as committed at its base, so a worker can't change its
/// own checks, with what `task.start` asked laid over it, and the problems reading the file.
pub async fn spec(repo: &str, base: &str, asked: &Asked) -> Result<(ChecksFile, Vec<String>), RpcError> {
    let (file, problems) = match git::show_file(repo, base, CHECKS_PATH).await? {
        Some(bytes) => parse_checks(&String::from_utf8_lossy(&bytes)),
        None => (ChecksFile::default(), Vec::new()),
    };
    let mut both = merged(&file, &asked_checks(asked));
    if let Some(rounds) = asked.qa_rounds {
        both.qa_rounds = rounds;
    }
    Ok((both, problems))
}

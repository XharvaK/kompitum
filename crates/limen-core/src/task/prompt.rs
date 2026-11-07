//! What a turn is told: the hand-off note, the brief, this repository's rules, and any follow-up.

use crate::rpc::RpcError;
use crate::task::types::{Asked, Task};
use crate::task::worker::Worker;

/// A turn's prompt, and what is worth saying about how it was built.
#[derive(Clone, Debug, Default)]
pub struct TurnPrompt {
    pub text: Vec<u8>,
    pub warnings: Vec<String>,
}

const FOLLOWUP: &str = "\n\n---\n\n# What is asked now\n\nAn earlier turn of this task has ended, and its changes are in \
                        your folder. This comes on top of the brief above:\n\n";

/// The session a follow-up continues: the last turn's, when this worker ran it and reported one.
pub fn resume_session(task: &Task, worker: Worker, has_followup: bool) -> Option<String> {
    if !has_followup {
        return None;
    }
    let last = task.turns.last()?;
    if last.worker != worker.key() {
        return None;
    }
    last.session.clone()
}

/// What this turn's worker reads. Resuming, the follow-up alone: its session has the rest.
pub async fn turn_prompt(
    _task: &Task,
    brief: &str,
    note: Option<&[u8]>,
    followup: Option<&[u8]>,
    resuming: bool,
) -> Result<TurnPrompt, RpcError> {
    if resuming {
        return Ok(TurnPrompt { text: followup.unwrap_or_default().to_vec(), warnings: Vec::new() });
    }
    let mut text = note.unwrap_or_default().to_vec();
    text.extend_from_slice(brief.as_bytes());
    if let Some(followup) = followup {
        text.extend_from_slice(FOLLOWUP.as_bytes());
        text.extend_from_slice(followup);
    }
    Ok(TurnPrompt { text, warnings: Vec::new() })
}

/// What is worth saying before a worker starts. These warn and never refuse.
pub async fn start_warnings(_repo: &str, _base: &str, _brief: &str, _scope: &[String], _asked: &Asked) -> Vec<String> {
    Vec::new()
}

//! `kom vault`: the notes agents leave each other in the shared vault.
//! The vault runs here, in this process, as the Python Kompitum does.

use super::*;
use crate::vault::{Message, Report, Vault};

#[derive(Subcommand)]
pub(crate) enum VaultCmd {
    /// The messages open for me, newest first.
    Open(AsArgs),
    /// Read one note by its vault path; this also leaves a read mark.
    Read(ReadArgs),
    /// Search the vault: every word must be in the note; the filters narrow it.
    Find(FindArgs),
    /// Write a message into my own folder, then commit and push it.
    Post(PostArgs),
    /// Set the status of one of my own notes: open, answered or superseded.
    Mark(MarkArgs),
    /// Close a note for me with no reply, leaving a done mark.
    Done(DoneArgs),
    /// Every message between me and another agent, both ways, oldest first.
    Thread(ThreadArgs),
    /// The messages added since the last commit this reader saw.
    Changes(AsArgs),
}

#[derive(clap::Args)]
pub(crate) struct AsArgs {
    /// Act as this agent, overriding KOMPITUM_AGENT.
    #[arg(long = "as", value_name = "AGENT")]
    pub name: Option<String>,
}

#[derive(clap::Args)]
pub(crate) struct ReadArgs {
    /// The note's vault path, such as manager/to/opus/2026-11-04_1826_living-brain.md.
    pub path: String,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

#[derive(clap::Args)]
pub(crate) struct FindArgs {
    /// Words that must all be in the note (case does not matter).
    pub words: Vec<String>,
    /// Only notes of this project (front matter, or the folder).
    #[arg(long, value_name = "P")]
    pub project: Option<String>,
    /// Only notes of this kind.
    #[arg(long, value_name = "K")]
    pub kind: Option<String>,
    /// Only notes with this status.
    #[arg(long, value_name = "S")]
    pub status: Option<String>,
    /// Only notes from this agent (front matter, or the folder).
    #[arg(long = "from", value_name = "A")]
    pub from_agent: Option<String>,
    /// The newest this many (default 20).
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub limit: usize,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

#[derive(clap::Args)]
pub(crate) struct PostArgs {
    /// The agent's folder to write into, or all.
    #[arg(long, value_name = "A")]
    pub to: String,
    /// request, report, decision, tried, question or answer.
    #[arg(long, value_name = "K")]
    pub kind: String,
    /// One line naming the note.
    #[arg(long, value_name = "T")]
    pub title: String,
    /// The body, in Markdown.
    #[arg(long, value_name = "TEXT", conflicts_with = "body_file", required_unless_present = "body_file")]
    pub body: Option<String>,
    /// The body's file, or - for standard input.
    #[arg(long = "body-file", value_name = "FILE", conflicts_with = "body")]
    pub body_file: Option<String>,
    /// The project the note belongs to.
    #[arg(long, value_name = "P")]
    pub project: Option<String>,
    /// A note this one answers (repeatable for one answer to several).
    #[arg(long = "re", value_name = "PATH")]
    pub re: Vec<String>,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

#[derive(clap::Args)]
pub(crate) struct MarkArgs {
    /// One of my own notes.
    pub path: String,
    /// open, answered or superseded.
    pub status: String,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

#[derive(clap::Args)]
pub(crate) struct DoneArgs {
    /// The note to close.
    pub path: String,
    /// Why it is closed, kept with the mark.
    #[arg(long, value_name = "TEXT")]
    pub why: Option<String>,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

#[derive(clap::Args)]
pub(crate) struct ThreadArgs {
    /// The other agent.
    pub agent: String,
    /// The newest this many messages (default 50).
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,
    #[command(flatten)]
    pub as_agent: AsArgs,
}

/// The `kom vault` verbs a worker may run: only those that look.
pub(crate) fn vault_as_a_worker(action: &VaultCmd) -> Result<(), RpcError> {
    match action {
        VaultCmd::Open(_) | VaultCmd::Read(_) | VaultCmd::Find(_) | VaultCmd::Thread(_) | VaultCmd::Changes(_) => {
            Ok(())
        }
        VaultCmd::Post(_) | VaultCmd::Mark(_) | VaultCmd::Done(_) => Err(worker_refused()),
    }
}

pub(crate) fn vault_cmd(action: VaultCmd, asked: bool) -> Result<Option<Out>, RpcError> {
    let _ = asked;
    Ok(Some(match action {
        VaultCmd::Open(a) => {
            let report = vault(&a.name)?.open()?;
            Out { json: report.json(), human: open_human(&report) }
        }
        VaultCmd::Read(a) => {
            let report = vault(&a.as_agent.name)?.read(&a.path)?;
            let mut human = problems_human(&report);
            if let Some(note) = &report.note {
                if !human.is_empty() {
                    human.push('\n');
                }
                human.push_str(&crate::vault::read_text_of(note));
            }
            Out { json: report.json(), human }
        }
        VaultCmd::Find(a) => {
            let report = vault(&a.as_agent.name)?.find(crate::vault::Query {
                words: a.words.join(" "),
                project: a.project.filter(|value| !value.is_empty()),
                kind: a.kind.filter(|value| !value.is_empty()),
                status: a.status.filter(|value| !value.is_empty()),
                from: a.from_agent.filter(|value| !value.is_empty()),
                limit: a.limit,
            })?;
            Out { json: report.json(), human: find_human(&report) }
        }
        VaultCmd::Post(a) => {
            let body = match (&a.body, &a.body_file) {
                (Some(body), _) => body.clone(),
                (_, Some(file)) => read_body(file)?,
                (None, None) => String::new(),
            };
            let written = vault(&a.as_agent.name)?.post(crate::vault::Post {
                to: a.to,
                kind: a.kind,
                title: a.title,
                body,
                project: a.project,
                re: a.re,
            })?;
            Out { json: written.json(), human: written.said }
        }
        VaultCmd::Mark(a) => {
            let written = vault(&a.as_agent.name)?.mark(&a.path, &a.status)?;
            Out { json: written.json(), human: written.said }
        }
        VaultCmd::Done(a) => {
            let written = vault(&a.as_agent.name)?.done(&a.path, a.why.as_deref())?;
            Out { json: written.json(), human: written.said }
        }
        VaultCmd::Thread(a) => {
            let report = vault(&a.as_agent.name)?.thread(&a.agent, a.limit)?;
            Out { json: report.json(), human: thread_human(&report, &a.agent) }
        }
        VaultCmd::Changes(a) => {
            let report = vault(&a.name)?.changes()?;
            Out { json: report.json(), human: changes_human(&report) }
        }
    }))
}

fn vault(agent: &Option<String>) -> Result<Vault, RpcError> {
    Vault::from_env(agent.as_deref())
}

fn read_body(file: &str) -> Result<String, RpcError> {
    if file == "-" {
        let mut bytes = Vec::new();
        std::io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|e| RpcError::new("usage", format!("cannot read standard input: {e}"), ""))?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let bytes = std::fs::read(file)
        .map_err(|e| RpcError::new("usage", format!("cannot read {file}: {e}"), "give a body file, or - for stdin"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn problems_human(report: &Report) -> String {
    report.problems.iter().map(|problem| format!("problem: {problem}")).collect::<Vec<_>>().join("\n")
}

fn open_human(report: &Report) -> String {
    let mut lines = Vec::new();
    let problems = problems_human(report);
    if !problems.is_empty() {
        lines.push(problems);
    }
    let mut old = 0;
    for note in &report.notes {
        if crate::vault::older_than_a_week(note) {
            old += 1;
        } else {
            lines.push(note.line());
        }
    }
    if old > 0 {
        lines.push(format!("{old} open notes older than 7 days"));
    }
    if report.notes.is_empty() {
        lines.push("Nothing is open.".into());
    }
    lines.join("\n")
}

fn find_human(report: &Report) -> String {
    let mut lines = Vec::new();
    let problems = problems_human(report);
    if !problems.is_empty() {
        lines.push(problems);
    }
    if report.notes.is_empty() {
        lines.push("Nothing in the vault matches.".into());
    }
    for note in &report.notes {
        let date = note.date().filter(|date| !date.is_empty());
        let when = date.as_deref().map(|date| date.get(..16).unwrap_or(date)).unwrap_or("(no date)");
        let kind = note.kind().filter(|kind| !kind.is_empty()).unwrap_or_else(|| "note".to_string());
        let mark = note.mark().map(|mark| format!(" · {mark}")).unwrap_or_default();
        lines.push(format!(
            "{when} {kind} · {} ({}){mark}\n  {}",
            note.title,
            note.path,
            note.snippet.as_deref().unwrap_or("")
        ));
    }
    lines.join("\n")
}

fn thread_human(report: &Report, other: &str) -> String {
    let mut lines = vec![problems_human(report)];
    if report.notes.is_empty() {
        lines.push(format!("Nothing between you and {other}."));
    }
    for note in &report.notes {
        lines.push(format!("{}\n\n{}", note.line(), note.body.trim_end()));
    }
    lines.retain(|line| !line.is_empty());
    lines.join("\n\n")
}

fn changes_human(report: &Report) -> String {
    let mut lines = vec![problems_human(report)];
    if report.notes.is_empty() {
        lines.push("Nothing new since the last check.".into());
    }
    lines.extend(report.notes.iter().map(Message::line));
    lines.retain(|line| !line.is_empty());
    lines.join("\n")
}

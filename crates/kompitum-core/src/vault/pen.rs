//! Writing the vault: a message into the sender's own folder, a status mark
//! on one of its notes, and the one-pending check a read uses before it
//! commits its own mark.

use super::clock::Moment;
use super::git;
use super::listing::{clean_path, drive_path, inside, read_text};
use super::notes::{self, KINDS, Meta};
use super::privacy;
use super::store::{BODY_MAX, Post, Vault, unavailable};
use crate::rpc::RpcError;
use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

pub(super) fn write_message(vault: &Vault, post: &Post, now: &Moment) -> Result<String, RpcError> {
    if !valid_name(&vault.agent) {
        return Err(RpcError::new(
            "usage",
            format!("\"{}\" is not an agent name: lower-case letters, digits and dashes.", vault.agent),
            "",
        ));
    }
    if !valid_name(&post.to) {
        return Err(RpcError::new(
            "usage",
            format!("\"{}\" is not an agent name: lower-case letters, digits and dashes, or all.", post.to),
            "",
        ));
    }
    if !KINDS.contains(&post.kind.as_str()) {
        if post.kind == notes::MEMORY {
            return Err(RpcError::new(
                "usage",
                "A memory is written only by kom vault remember; post carries messages, not memories.",
                "",
            ));
        }
        return Err(RpcError::new("usage", format!("kind is one of: {}.", KINDS.join(", ")), ""));
    }
    let title = post.title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(RpcError::new("usage", "A message needs a title.", ""));
    }
    if post.body.trim().is_empty() {
        return Err(RpcError::new("usage", "A message needs a body.", ""));
    }
    if post.body.chars().count() > BODY_MAX {
        return Err(RpcError::new("usage", format!("A message body is at most {BODY_MAX} characters."), ""));
    }
    let mut replies = Vec::new();
    for path in &post.re {
        let path = clean_path(path);
        if path.is_empty() {
            return Err(RpcError::new("usage", "re is the path of a note, or a list of paths.", ""));
        }
        let segments: Vec<&str> = path.split('/').collect();
        if segments.contains(&"..") || path.starts_with('/') || drive_path(&path) {
            return Err(RpcError::new("usage", format!("re: {path} is not a path inside the vault."), ""));
        }
        if !vault.root.join(&path).exists() {
            return Err(RpcError::new("usage", format!("re: names {path}, which is not in the vault."), ""));
        }
        if !replies.contains(&path) {
            replies.push(path);
        }
    }
    if replies.len() > 1
        && let Some(path) = replies.iter().find(|path| path.contains(',') || path.contains('[') || path.contains(']'))
    {
        return Err(RpcError::new(
            "usage",
            format!("re: {path} holds a comma or a bracket, so it cannot go in a list of paths."),
            "",
        ));
    }
    let kinds = privacy::private_kinds(&format!("{title}\n{}", post.body));
    if !kinds.is_empty() {
        return Err(RpcError::new(
            "refused",
            format!("The message holds what looks like {}. Nothing was written.", kinds.join(", ")),
            "",
        ));
    }

    let mut meta = Meta::default();
    meta.set("from", notes::Value::Text(vault.agent.clone()));
    meta.set("to", notes::Value::List(vec![post.to.clone()]));
    meta.set("kind", notes::Value::Text(post.kind.clone()));
    if let Some(project) = post.project.as_ref().filter(|project| !project.is_empty()) {
        meta.set("project", notes::Value::Text(project.clone()));
    }
    meta.set("date", notes::Value::Text(now.note_stamp()));
    meta.set("title", notes::Value::Text(title.clone()));
    if replies.len() == 1 {
        meta.set("re", notes::Value::Text(replies[0].clone()));
    } else if replies.len() > 1 {
        meta.set("re", notes::Value::List(replies));
    }
    let status = if post.kind == "answer" || post.kind == "report" { "answered" } else { "open" };
    meta.set("status", notes::Value::Text(status.to_string()));

    let folder = vault.root.join(&vault.agent).join("to").join(&post.to);
    std::fs::create_dir_all(&folder).map_err(|e| unavailable(format!("cannot make {}: {e}", folder.display())))?;
    let base = format!("{}_{}", now.file_stamp(), notes::slug(&title));
    let mut name = format!("{base}.md");
    let mut number = 2;
    while folder.join(&name).exists() {
        name = format!("{base}-{number}.md");
        number += 1;
    }
    let text = notes::format_note(&meta, &post.body);
    std::fs::write(folder.join(&name), text.as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", folder.join(&name).display())))?;
    Ok(format!("{}/to/{}/{}", vault.agent, post.to, name))
}

pub(super) fn mark_note(root: &Path, path: &str, status: &str) -> Result<(), RpcError> {
    inside(root, path)?;
    let full = root.join(path);
    let note = notes::parse_note(&read_text(&full)?);
    let mut meta = note.meta.clone();
    meta.set("status", notes::Value::Text(status.to_string()));
    std::fs::write(&full, notes::format_note(&meta, &note.body).as_bytes())
        .map_err(|e| unavailable(format!("cannot write {}: {e}", full.display())))
}

pub(super) fn valid_name(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,39}$").expect("an agent name"));
    re.is_match(name)
}

pub(super) fn only_pending(root: &Path, ours: &str) -> bool {
    let paths = git::status_paths(root);
    !paths.is_empty() && paths.iter().all(|path| path == ours)
}

#[cfg(test)]
mod tests {
    use super::super::clock;
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kom-pen-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }

    fn vault(root: &Path) -> Vault {
        Vault { root: root.to_path_buf(), agent: "opus".into() }
    }

    fn note(from: &str, to: &str, kind: &str, date: &str, title: &str) -> String {
        format!(
            "---\nfrom: {from}\nto: [{to}]\nkind: {kind}\ndate: {date}\ntitle: {title}\n---\n\n# {title}\n\nBody.\n"
        )
    }

    #[test]
    fn post_writes_into_my_folder_with_a_unique_name_and_the_python_front_matter() {
        let root = scratch("post");
        write(&root, "manager/to/opus/a.md", &note("manager", "opus", "request", "2026-11-01", "A"));
        let v = vault(&root);
        let post = Post {
            to: "manager".into(),
            kind: "answer".into(),
            title: "Key: moved to the vault".into(),
            body: "Done.".into(),
            project: Some("myapp".into()),
            re: vec!["manager/to/opus/a.md".into()],
        };
        let moment = clock::Moment::now();
        let first = write_message(&v, &post, &moment).unwrap();
        let second = write_message(&v, &post, &moment).unwrap();
        assert!(first.starts_with("opus/to/manager/") && first.ends_with("_key-moved-to-the-vault.md"), "{first}");
        assert_eq!(second, first.replace(".md", "-2.md"));
        let text = std::fs::read_to_string(root.join(&first)).unwrap();
        let parsed = notes::parse_note(&text);
        let keys: Vec<&str> = parsed.meta.0.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(keys, ["from", "to", "kind", "project", "date", "title", "re", "status"]);
        assert_eq!(parsed.meta.text("from").as_deref(), Some("opus"));
        assert_eq!(parsed.meta.list("to"), ["manager"]);
        assert_eq!(parsed.meta.text("re").as_deref(), Some("manager/to/opus/a.md"));
        assert_eq!(parsed.meta.text("status").as_deref(), Some("answered"));
        assert_eq!(parsed.body, "Done.\n");

        let refused = write_message(
            &v,
            &Post { to: "../x".into(), kind: "report".into(), title: "t".into(), body: "b".into(), ..Post::default() },
            &moment,
        )
        .unwrap_err();
        assert!(refused.message.contains("not an agent name"), "{}", refused.message);
        let refused = write_message(
            &v,
            &Post {
                to: "manager".into(),
                kind: "memory".into(),
                title: "t".into(),
                body: "b".into(),
                ..Post::default()
            },
            &moment,
        )
        .unwrap_err();
        assert!(refused.message.contains("remember"), "{}", refused.message);
        let secret = ["gh", "p_", "aB3dE6gH9jK2mN5pQ8sT1vW4"].concat();
        let refused = write_message(
            &v,
            &Post {
                to: "manager".into(),
                kind: "report".into(),
                title: "t".into(),
                body: format!("the token {secret}"),
                ..Post::default()
            },
            &moment,
        )
        .unwrap_err();
        assert!(refused.message.contains("a GitHub token"), "{}", refused.message);
        assert!(refused.message.contains("Nothing was written"), "{}", refused.message);

        write(&root, "manager/to/opus/a,b.md", &note("manager", "opus", "request", "2026-11-01", "Comma"));
        for re in ["../outside.md", "manager/../../outside.md"] {
            let refused = write_message(
                &v,
                &Post {
                    to: "manager".into(),
                    kind: "answer".into(),
                    title: "T".into(),
                    body: "B".into(),
                    re: vec![re.into()],
                    ..Post::default()
                },
                &moment,
            )
            .unwrap_err();
            assert!(refused.message.contains("inside the vault"), "{}", refused.message);
        }
        let refused = write_message(
            &v,
            &Post {
                to: "manager".into(),
                kind: "answer".into(),
                title: "T".into(),
                body: "B".into(),
                re: vec!["manager/to/opus/missing.md".into()],
                ..Post::default()
            },
            &moment,
        )
        .unwrap_err();
        assert!(refused.message.contains("not in the vault"), "{}", refused.message);
        let refused = write_message(
            &v,
            &Post {
                to: "manager".into(),
                kind: "answer".into(),
                title: "T".into(),
                body: "B".into(),
                re: vec!["manager/to/opus/a,b.md".into(), "manager/to/opus/a.md".into()],
                ..Post::default()
            },
            &moment,
        )
        .unwrap_err();
        assert!(refused.message.contains("holds a comma or a bracket"), "{}", refused.message);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mark_changes_only_the_status_of_a_note_it_can_read() {
        let root = scratch("mark");
        write(&root, "opus/to/manager/a.md", "---\nfrom: opus\nto: [manager]\nkind: report\ntitle: A\n---\n\nBody.\n");
        mark_note(&root, "opus/to/manager/a.md", "superseded").unwrap();
        let text = std::fs::read_to_string(root.join("opus/to/manager/a.md")).unwrap();
        let parsed = notes::parse_note(&text);
        assert_eq!(parsed.meta.text("status").as_deref(), Some("superseded"));
        assert_eq!(parsed.meta.text("title").as_deref(), Some("A"));
        assert_eq!(parsed.body, "Body.\n");
        assert!(inside(&root, "../outside.md").is_err());
        assert!(inside(&root, "C:/outside.md").is_err());
        assert!(inside(&root, "opus/to/manager/missing.md").is_err());
        let v = vault(&root);
        let refused = v.mark("manager/to/opus/a.md", "answered").unwrap_err();
        assert!(refused.message.contains("not yours"), "{}", refused.message);
        let refused = v.mark("opus/to/manager/a.md", "done").unwrap_err();
        assert!(refused.message.contains("status is one of"), "{}", refused.message);
        std::fs::remove_dir_all(root).unwrap();
    }
}

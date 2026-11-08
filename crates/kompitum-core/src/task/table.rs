//! The table: who may accept, cancel or discard whose task.
//! These stop accidents and wrong defaults, not a security boundary: every
//! caller is the same OS account.

use crate::rpc::RpcError;
use crate::task::types::Task;

/// The caller's word. A task started before `by` counts as another caller's.
pub fn caller(by: Option<&str>) -> &str {
    by.map(str::trim).filter(|word| !word.is_empty()).unwrap_or("owner")
}

/// A caller as a person reads it; an old record that names none is `—`.
pub fn by_word(by: Option<&str>) -> &str {
    by.map(str::trim).filter(|word| !word.is_empty()).unwrap_or("—")
}

fn who(task: &Task) -> &str {
    task.by.as_deref().map(str::trim).filter(|word| !word.is_empty()).unwrap_or("another caller")
}

/// A manager accepts only with the Owner's words; everyone else may pass them too.
pub fn accept_refusal(task: &Task, by: Option<&str>, owner_said: Option<&str>) -> Option<RpcError> {
    if caller(by) != "manager" || owner_said_words(owner_said).is_some() {
        return None;
    }
    Some(RpcError::new(
        "refused",
        format!("{}: a manager accepts a task only with the Owner's words", task.name),
        "pass --owner-said \"<the Owner's words>\"",
    ))
}

/// The words when they are words, not an empty flag.
pub fn owner_said_words(owner_said: Option<&str>) -> Option<&str> {
    owner_said.map(str::trim).filter(|words| !words.is_empty())
}

/// Opus and Manager may change another caller's task only with `yes`;
/// the Owner always may, and anyone may change their own.
pub fn change_refusal(task: &Task, by: Option<&str>, yes: bool) -> Option<RpcError> {
    let caller = caller(by);
    let started = task.by.as_deref().map(str::trim).filter(|word| !word.is_empty());
    if !matches!(caller, "opus" | "manager") || yes || started == Some(caller) {
        return None;
    }
    Some(RpcError::new(
        "refused",
        format!("{} was started by {}", task.name, who(task)),
        "ask them or the Owner, then pass --yes",
    ))
}

/// `Owner: "<words>". <review>`, or the words alone when there is no review.
pub fn review_with_owner(owner_said: Option<&str>, review: Option<&str>) -> Option<String> {
    let review = review.map(str::trim).filter(|text| !text.is_empty());
    let Some(said) = owner_said_words(owner_said) else { return review.map(String::from) };
    Some(match review {
        Some(review) => format!("Owner: \"{said}\". {review}"),
        None => format!("Owner: \"{said}\"."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(by: Option<&str>) -> Task {
        Task { name: "fix-1".into(), by: by.map(String::from), ..Default::default() }
    }

    #[test]
    fn a_manager_accepts_only_with_the_owners_words() {
        let some = task(Some("manager"));
        let refused = accept_refusal(&some, Some("manager"), None).unwrap();
        assert_eq!(refused.kind, "refused");
        assert!(refused.hint.contains("--owner-said"), "{}", refused.hint);
        assert!(accept_refusal(&some, Some("manager"), Some("ship it")).is_none());
        assert!(accept_refusal(&some, Some("manager"), Some("  ")).is_some(), "empty words are no words");
        assert!(accept_refusal(&some, Some("opus"), None).is_none());
        assert!(accept_refusal(&some, Some("owner"), None).is_none());
        assert!(accept_refusal(&some, None, None).is_none(), "no caller reads as the Owner");
    }

    #[test]
    fn another_callers_task_is_changed_only_with_yes_or_by_the_owner() {
        let theirs = task(Some("opus"));
        let refused = change_refusal(&theirs, Some("manager"), false).unwrap();
        assert_eq!(refused.kind, "refused");
        assert!(refused.message.contains("opus"), "{}", refused.message);
        assert!(refused.hint.contains("--yes"), "{}", refused.hint);
        assert!(change_refusal(&theirs, Some("manager"), true).is_none());
        assert!(change_refusal(&theirs, Some("opus"), false).is_none(), "its own task is not another's");
        assert!(change_refusal(&theirs, Some("owner"), false).is_none());
        let old = task(None);
        assert!(change_refusal(&old, Some("opus"), false).is_some(), "a record naming no one is another's");
        assert!(change_refusal(&old, Some("owner"), false).is_none());
    }

    #[test]
    fn the_owner_words_come_before_the_review() {
        assert_eq!(review_with_owner(Some("ship it"), Some("sound")).as_deref(), Some("Owner: \"ship it\". sound"));
        assert_eq!(review_with_owner(Some("ship it"), None).as_deref(), Some("Owner: \"ship it\"."));
        assert_eq!(review_with_owner(None, Some("sound")).as_deref(), Some("sound"));
        assert_eq!(review_with_owner(None, None), None);
        assert_eq!(by_word(None), "—");
        assert_eq!(by_word(Some("opus")), "opus");
    }
}

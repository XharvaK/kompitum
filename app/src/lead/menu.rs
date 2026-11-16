//! Reading Claude Code's own menus off the lead terminal's screen.

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScreenMenu {
    pub title: String,
    pub body: Vec<MenuBlock>,
    pub rows: Vec<MenuRow>,
    pub pointer: usize,
    pub notes: Vec<MenuBlock>,
    pub actions: Vec<MenuAction>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuBlock {
    pub text: String,
    pub mono: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuRow {
    pub label: String,
    pub detail: String,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuAction {
    pub key: String,
    pub label: String,
}

/// The direction and count of presses that walk the pointer from `pointer` to `row`.
pub fn menu_steps(pointer: usize, row: usize) -> (&'static str, usize) {
    if row > pointer {
        ("down", row - pointer)
    } else if row < pointer {
        ("up", pointer - row)
    } else {
        ("", 0)
    }
}

/// Reads one of Claude Code's own menus from a terminal screen, or None when the screen
/// holds no single menu.
pub fn menu_from_screen(text: &str) -> Option<ScreenMenu> {
    let lines: Vec<String> = text.lines().map(|line| line.trim_end().to_string()).collect();
    let footer_index = lines.iter().rposition(|line| !line.trim().is_empty())?;
    let actions = footer_actions(&lines[footer_index])?;
    let rule_index = (0..footer_index)
        .rev()
        .take(45)
        .find(|index| is_rule(&lines[*index]))?;
    let (pointer_index, pointer_indent, pointer_text) =
        pointer_in(&lines[rule_index + 1..footer_index], rule_index)?;
    let title_index = (rule_index + 1..footer_index).find(|index| !lines[*index].trim().is_empty())?;
    let title = lines[title_index].trim().to_owned();
    let (first, last, rows, pointer) = read_rows(&lines, rule_index, footer_index, pointer_index, pointer_indent, pointer_text)?;
    Some(ScreenMenu {
        title,
        body: blocks(&lines[title_index + 1..first]),
        rows,
        pointer,
        notes: notes_blocks(&lines[last + 1..footer_index]),
        actions,
    })
}

fn upper_first(words: &str) -> String {
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn footer_actions(line: &str) -> Option<Vec<MenuAction>> {
    let mut enter = None;
    let mut esc = false;
    let mut rest = Vec::new();
    for part in line.trim().split(" · ") {
        let Some((key, words)) = part.split_once(" to ") else {
            continue;
        };
        let key = key.trim();
        let words = words.trim();
        if key == "Type" || key.contains('←') || key.contains('→') || key.contains("Enter/") {
            return None;
        }
        let action = match key {
            "Enter" => Some(MenuAction {
                key: "enter".into(),
                label: upper_first(words),
            }),
            "Esc" => {
                esc = true;
                Some(MenuAction {
                    key: "esc".into(),
                    label: upper_first(words),
                })
            }
            _ if key.chars().count() == 1
                && key
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()) =>
            {
                Some(MenuAction {
                    key: key.to_owned(),
                    label: upper_first(words),
                })
            }
            _ => None,
        };
        if let Some(action) = action {
            if action.key == "enter" {
                enter = Some(action);
            } else {
                rest.push(action);
            }
        }
    }
    if !esc {
        return None;
    }
    let mut actions = vec![enter.unwrap_or(MenuAction {
        key: "enter".into(),
        label: String::new(),
    })];
    actions.extend(rest);
    Some(actions)
}

/// A region rule: at least twenty `─` or `▔` characters. Dashed rules (`╌`) never count.
fn is_rule(line: &str) -> bool {
    let line = line.trim();
    let Some(first) = line.chars().next() else {
        return false;
    };
    matches!(first, '─' | '▔') && line.chars().count() >= 20 && line.chars().all(|ch| ch == first)
}

fn is_dashed(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().all(|ch| ch == '╌')
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count()
}

fn pointer_of(line: &str) -> Option<(usize, &str)> {
    let indent = indent_of(line);
    let rest = line[indent..].strip_prefix("❯ ")?;
    Some((indent, rest))
}

fn pointer_in(region: &[String], rule_index: usize) -> Option<(usize, usize, &str)> {
    let mut found = None;
    for (offset, line) in region.iter().enumerate() {
        if let Some((indent, text)) = pointer_of(line) {
            if found.is_some() {
                return None;
            }
            found = Some((rule_index + 1 + offset, indent, text));
        }
    }
    let (index, indent, text) = found?;
    // The input box's own prompt sits just under its top rule.
    if index == rule_index + 1 {
        return None;
    }
    Some((index, indent, text))
}

/// The rows and the pointer's index in them.
fn read_rows(
    lines: &[String],
    rule_index: usize,
    footer_index: usize,
    pointer_index: usize,
    pointer_indent: usize,
    pointer_text: &str,
) -> Option<(usize, usize, Vec<MenuRow>, usize)> {
    let deep = pointer_indent + 2;
    let mut first = pointer_index;
    while first > rule_index + 1 {
        let above = first - 1;
        if lines[above].trim().is_empty() {
            // Claude separates some lists from their rows with a blank line; keep looking for the list.
            first = above;
            continue;
        }
        if indent_of(&lines[above]) < deep {
            break;
        }
        first = above;
    }
    let mut last = pointer_index;
    while last + 1 < footer_index {
        let below = last + 1;
        if lines[below].trim().is_empty() || indent_of(&lines[below]) < deep {
            break;
        }
        last = below;
    }
    let span: Vec<usize> = (first..=last).collect();
    let base = span
        .iter()
        .filter(|index| **index != pointer_index && !lines[**index].trim().is_empty())
        .map(|index| indent_of(&lines[*index]))
        .min()
        .unwrap_or(deep);
    let mut texts: Vec<(usize, String)> = Vec::new();
    for index in span {
        if lines[index].trim().is_empty() && index != pointer_index {
            continue;
        }
        let text = if index == pointer_index {
            pointer_text.trim()
        } else {
            lines[index].trim()
        };
        let new_row = index == pointer_index || indent_of(&lines[index]) == base || starts_with_number(text);
        if new_row || texts.is_empty() {
            texts.push((index, text.to_owned()));
        } else if let Some((_, row)) = texts.last_mut() {
            row.push(' ');
            row.push_str(text);
        }
    }
    if texts.len() < 2 || texts.len() > 12 {
        return None;
    }
    let pointer = texts.iter().position(|(index, _)| *index == pointer_index)?;
    let rows = texts.into_iter().map(|(_, text)| row(text)).collect();
    Some((first, last, rows, pointer))
}

fn starts_with_number(text: &str) -> bool {
    let mut seen = false;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            seen = true;
        } else {
            return seen && ch == '.';
        }
    }
    false
}

fn row(text: String) -> MenuRow {
    let mut text = text.trim().to_owned();
    let mut current = false;
    if let Some(rest) = text.strip_prefix('✔') {
        current = true;
        text = rest.trim_start().to_owned();
    }
    text = strip_number(&text).unwrap_or(&text).trim_start().to_owned();
    if let Some(rest) = text.strip_prefix('▸') {
        text = rest.trim_start().to_owned();
    }
    if let Some(at) = text.find('✔') {
        current = true;
        text.replace_range(at..at + '✔'.len_utf8(), "");
    }
    let mut parts = text
        .split("  ")
        .map(str::trim)
        .filter(|part| !part.is_empty());
    let label = parts.next().unwrap_or_default().to_owned();
    let detail = parts.collect::<Vec<_>>().join(" · ");
    MenuRow {
        label,
        detail,
        current,
    }
}

fn strip_number(text: &str) -> Option<&str> {
    let end = text.find('.')?;
    if end == 0 || !text[..end].chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(&text[end + 1..])
}

fn blocks(lines: &[String]) -> Vec<MenuBlock> {
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if is_dashed(&lines[index]) {
            let Some(end) = (index + 1..lines.len()).find(|at| is_dashed(&lines[*at])) else {
                break;
            };
            let text = lines[index + 1..end]
                .iter()
                .take(12)
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                blocks.push(MenuBlock { text, mono: true });
            }
            index = end + 1;
            continue;
        }
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let mut parts = Vec::new();
        while index < lines.len() && !lines[index].trim().is_empty() && !is_dashed(&lines[index]) {
            parts.push(lines[index].trim().to_owned());
            index += 1;
        }
        let text = parts.join(" ");
        if !text.is_empty() {
            blocks.push(MenuBlock { text, mono: false });
        }
    }
    blocks
}

fn notes_blocks(lines: &[String]) -> Vec<MenuBlock> {
    blocks(lines)
        .into_iter()
        .map(|mut block| {
            if !block.mono {
                block.text = clean_note(&block.text);
            }
            block
        })
        .filter(|block| !block.text.is_empty())
        .collect()
}

/// Drops a note's trailing arrows-to-adjust part and its `(ctrl+… to …)` part.
fn clean_note(text: &str) -> String {
    let mut text = text.to_owned();
    while let Some(start) = text.find("(ctrl+") {
        let Some(rest) = text.get(start..) else { break };
        let Some(to) = rest.find(" to ") else { break };
        let Some(end) = rest[to..].find(')').map(|at| start + to + at) else {
            break;
        };
        text.replace_range(start..=end, "");
    }
    for marker in ["←/→ to ", "← to ", "→ to ", "←→ to "] {
        if let Some(at) = text.rfind(marker) {
            text.truncate(at);
        }
    }
    text.trim().to_owned()
}

#[cfg(test)]
#[path = "menu/tests.rs"]
mod tests;

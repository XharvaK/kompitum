//! `file.read`, `file.write`, `file.edit`: an agent's hands on files on the
//! machine that has them, whatever harness it runs in.

use crate::api::{Edit, FileAt, FileEditParams, FileReadParams, FileReadResult, FileResult, FileWriteParams};
use crate::rpc::RpcError;
use std::path::{Path, PathBuf};

/// Raw bytes one read gives or one write takes (inline in one message).
pub const MAX_BYTES: usize = 8 << 20;
/// Files read whole for read and edit.
const MAX_FILE: u64 = 64 << 20;

pub fn read(p: FileReadParams, from_afar: bool) -> Result<FileReadResult, RpcError> {
    let path = resolve(&p.at, from_afar)?;
    let text = text_of(&path)?;
    lines(&text, p.offset.unwrap_or(1), p.limit.unwrap_or(usize::MAX), MAX_BYTES)
}

pub fn write(p: FileWriteParams, from_afar: bool) -> Result<FileResult, RpcError> {
    let bytes = crate::put::decode(&p.data).ok_or_else(|| RpcError::invalid_params("data is not base64"))?;
    if bytes.len() > MAX_BYTES {
        return Err(RpcError::invalid_params(format!("at most {} MB per write", MAX_BYTES >> 20)));
    }
    let path = resolve(&p.at, from_afar)?;
    swap_in(&path, &bytes)
}

pub fn edit(p: FileEditParams, from_afar: bool) -> Result<FileResult, RpcError> {
    let path = resolve(&p.at, from_afar)?;
    let edited = apply(&text_of(&path)?, &p.edits)?;
    swap_in(&path, edited.as_bytes())
}

/// Relative paths start at `cwd`, as exec's programs do, and stay inside `root`:
/// a path or link that leads out is refused.
fn resolve(at: &FileAt, from_afar: bool) -> Result<PathBuf, RpcError> {
    let start = PathBuf::from(crate::exec::folder(at.cwd.as_deref(), from_afar));
    let path = start.join(&at.path);
    let root = at.root.as_ref().map_or_else(|| start.clone(), |r| start.join(r));
    if !real(&path)?.starts_with(real(&root)?) {
        return Err(RpcError::new(
            "outside_root",
            format!("{} is outside {}", path.display(), root.display()),
            "say where this call may reach with --root",
        ));
    }
    Ok(path)
}

/// Where `path` really is: every link on the way followed, even one whose target isn't
/// there yet, and `..` taken after the link before it, as the OS does. On Windows, case and
/// short names are settled by canonicalizing the longest part that exists.
fn real(path: &Path) -> Result<PathBuf, RpcError> {
    /// One component, owned: a link's target is walked by the same rules.
    enum Step {
        Prefix(std::ffi::OsString),
        Root,
        Parent,
        Name(std::ffi::OsString),
    }
    fn steps(path: &Path) -> Vec<Step> {
        use std::path::Component;
        path.components()
            .filter_map(|c| match c {
                Component::Prefix(p) => Some(Step::Prefix(p.as_os_str().to_os_string())),
                Component::RootDir => Some(Step::Root),
                Component::ParentDir => Some(Step::Parent),
                Component::Normal(name) => Some(Step::Name(name.to_os_string())),
                Component::CurDir => None,
            })
            .collect()
    }
    let mut resolved = PathBuf::new();
    let mut todo = steps(path);
    todo.reverse(); // pop takes them in path order
    let mut links = 0;
    while let Some(step) = todo.pop() {
        match step {
            Step::Prefix(prefix) => resolved.push(prefix),
            Step::Root => resolved.push(std::path::MAIN_SEPARATOR.to_string()),
            Step::Parent => drop(resolved.pop()),
            Step::Name(name) => {
                let next = resolved.join(name);
                match std::fs::read_link(&next) {
                    Ok(target) => {
                        // A link's target is taken apart again: its own links, and its `..`.
                        links += 1;
                        if links > 40 {
                            return Err(RpcError::invalid_params(format!(
                                "too many links on the way to {}",
                                path.display()
                            )));
                        }
                        let mut rest = steps(&target);
                        rest.reverse();
                        todo.extend(rest);
                    }
                    Err(_) => resolved = next,
                }
            }
        }
    }
    // Windows keeps case and short names as written until a file exists there.
    let mut existing = resolved.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(mut settled) = existing.canonicalize() {
            for part in rest.iter().rev() {
                settled.push(part);
            }
            return Ok(settled);
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return Ok(resolved),
        }
    }
}

fn io(path: &Path, e: std::io::Error) -> RpcError {
    match e.kind() {
        std::io::ErrorKind::NotFound => {
            RpcError::new("not_found", format!("no file {}", path.display()), "check the path, or lc exec ls")
        }
        _ => RpcError::new("io", format!("{}: {e}", path.display()), ""),
    }
}

/// A regular file's text: a pipe or device could block or never end.
fn text_of(path: &Path) -> Result<String, RpcError> {
    use std::io::Read;
    // Checked before opening: opening a pipe already waits for a writer.
    if !std::fs::metadata(path).map_err(|e| io(path, e))?.is_file() {
        return Err(RpcError::invalid_params(format!("{} is not a regular file", path.display())));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| io(path, e))?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io(path, e))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(RpcError::invalid_params(format!("{} is over {} MB", path.display(), MAX_FILE >> 20)));
    }
    String::from_utf8(bytes).map_err(|_| {
        RpcError::new("not_text", format!("{} is not UTF-8 text", path.display()), "for bytes: lc exec cat")
    })
}

/// Lines `first..` of `text`, at most `limit` of them and `max` bytes as
/// JSON (whole lines; a control character takes six), so a reply always
/// fits in a frame.
fn lines(text: &str, first: usize, limit: usize, max: usize) -> Result<FileReadResult, RpcError> {
    let total = text.split_inclusive('\n').count();
    if first == 0 || (first > total && first > 1) {
        return Err(RpcError::invalid_params(format!("offset {first}: the file has {total} lines")));
    }
    let json_len =
        |line: &str| line.len() + 5 * line.bytes().filter(|b| *b < 0x20 || *b == b'"' || *b == b'\\').count();
    let (mut out, mut size) = (String::new(), 0);
    for line in text.split_inclusive('\n').skip(first - 1).take(limit) {
        size += json_len(line);
        if size > max {
            break;
        }
        out.push_str(line);
    }
    let given = out.split_inclusive('\n').count();
    if given == 0 && limit > 0 && first <= total {
        return Err(RpcError::invalid_params(format!("line {first} is over {} MB", max >> 20)));
    }
    Ok(FileReadResult { text: out, first, last: first + given - 1, lines: total })
}

/// All edits, each found exactly once in `text` as it is, or none.
fn apply(text: &str, edits: &[Edit]) -> Result<String, RpcError> {
    if edits.is_empty() {
        return Err(RpcError::invalid_params("no edits"));
    }
    let which = |i: usize| if edits.len() == 1 { "the old text".to_string() } else { format!("edit {}", i + 1) };
    let failed = |message: String, hint: &str| RpcError::new("edit_failed", message, hint);
    let mut spans = Vec::new();
    for (i, e) in edits.iter().enumerate() {
        if e.old.is_empty() {
            return Err(RpcError::invalid_params(format!("{}: old is empty", which(i))));
        }
        match found(text, &e.old) {
            (_, 0) => return Err(failed(format!("{} is not in the file", which(i)), "read it again: exact text")),
            (at, 1) => spans.push((at, at + e.old.len(), i)),
            (_, n) => return Err(failed(format!("{} is in the file {n} times", which(i)), "add lines around it")),
        }
    }
    spans.sort();
    if let Some(w) = spans.windows(2).find(|w| w[1].0 < w[0].1) {
        return Err(failed(format!("edits {} and {} overlap", w[0].2 + 1, w[1].2 + 1), "make them one edit"));
    }
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for (start, end, i) in spans {
        out.push_str(&text[done..start]);
        out.push_str(&edits[i].new);
        done = end;
    }
    out.push_str(&text[done..]);
    Ok(out)
}

/// Where `old` first starts in `text`, and how often it is there, overlapping
/// finds too (`aa` is twice in `aaa`).
fn found(text: &str, old: &str) -> (usize, usize) {
    let step = old.chars().next().map_or(1, char::len_utf8);
    let (mut first, mut n, mut from) = (0, 0, 0);
    while let Some(at) = text[from..].find(old) {
        if n == 0 {
            first = from + at;
        }
        n += 1;
        from += at + step;
    }
    (first, n)
}

/// The file a write lands in: through symlinks, even to a target not there yet.
fn through_links(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::read_link(&path) {
            Ok(to) => path = path.parent().map_or(to.clone(), |dir| dir.join(&to)),
            Err(_) => break,
        }
    }
    path
}

/// Writes beside the file and renames it in: a reader sees old or new, never
/// half. Through a symlink to its target; an existing file keeps its mode.
fn swap_in(path: &Path, bytes: &[u8]) -> Result<FileResult, RpcError> {
    let path = through_links(path);
    let dir = path.parent().ok_or_else(|| RpcError::invalid_params("a file, not a folder"))?;
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.lc", crate::platform::random_hex()));
    let mode = std::fs::metadata(&path).ok().map(|m| m.permissions());
    let mut new = std::fs::OpenOptions::new();
    new.write(true).create_new(true);
    // Others never see the new content before it has the old file's mode.
    #[cfg(unix)]
    if mode.is_some() {
        std::os::unix::fs::OpenOptionsExt::mode(&mut new, 0o600);
    }
    let written = new
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, bytes))
        .and_then(|()| mode.map_or(Ok(()), |m| std::fs::set_permissions(&tmp, m)))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(&path, e));
    }
    Ok(FileResult { path: path.display().to_string(), bytes: bytes.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edits(pairs: &[(&str, &str)]) -> Vec<Edit> {
        pairs.iter().map(|(old, new)| Edit { old: old.to_string(), new: new.to_string() }).collect()
    }

    #[test]
    fn edits_each_match_once_in_the_original_and_never_overlap() {
        let text = "a b c\nd b e\n";
        assert_eq!(apply(text, &edits(&[("a", "A"), ("e\n", "E\n")])).unwrap(), "A b c\nd b E\n");
        assert_eq!(apply(text, &edits(&[("a", "b"), ("b c", "x")])).unwrap(), "b x\nd b e\n", "matched as it was");
        assert_eq!(apply("aaa", &edits(&[("aa", "X")])).unwrap_err().message, "the old text is in the file 2 times");
        let kind = |pairs: &[(&str, &str)]| {
            let e = apply(text, &edits(pairs)).unwrap_err();
            (e.kind, e.message)
        };
        assert_eq!(kind(&[("z", "")]), ("edit_failed".into(), "the old text is not in the file".into()));
        assert_eq!(kind(&[("a", "A"), (" b ", "")]), ("edit_failed".into(), "edit 2 is in the file 2 times".into()));
        assert_eq!(kind(&[("a b", ""), ("b c", "")]), ("edit_failed".into(), "edits 1 and 2 overlap".into()));
        assert_eq!(kind(&[("", "x")]).0, "invalid_params");
    }

    #[test]
    fn reads_give_whole_lines_from_an_offset_within_a_byte_cap() {
        let r = lines("1\n2\n3\n4", 2, 2, 100).unwrap();
        assert_eq!((r.text.as_str(), r.first, r.last, r.lines), ("2\n3\n", 2, 3, 4));
        let r = lines("1\n2\n3\n4", 1, usize::MAX, 15).unwrap();
        assert_eq!((r.text.as_str(), r.last), ("1\n2\n", 2), "cut at a line");
        let r = lines("1\n\u{1}\n", 1, usize::MAX, 9).unwrap();
        assert_eq!(r.text, "1\n", "its JSON size counts: \\u0001\\n is 12 bytes");
        assert_eq!(lines("\u{1}\u{1}\n", 1, usize::MAX, 9).err().map(|e| e.kind), Some("invalid_params".into()));
        let r = lines("", 1, usize::MAX, 100).unwrap();
        assert_eq!((r.text.as_str(), r.first, r.last, r.lines), ("", 1, 0, 0));
        assert_eq!(lines("1\n", 3, 1, 100).err().map(|e| e.kind), Some("invalid_params".into()));
    }

    fn at(cwd: &Path, path: &str, root: Option<&Path>) -> FileAt {
        FileAt {
            path: path.into(),
            on: None,
            cwd: Some(cwd.display().to_string()),
            root: root.map(|r| r.display().to_string()),
        }
    }

    #[test]
    fn a_file_call_stays_inside_its_root_and_may_say_where_that_is() {
        let dir = std::env::temp_dir().join(format!("lc-files-{}", crate::platform::random_hex()));
        let inside = dir.join("root").join("sub");
        std::fs::create_dir_all(&inside).unwrap();
        let kind = |path: &str, root: Option<&Path>| resolve(&at(&inside, path, root), false).err().map(|e| e.kind);

        assert!(resolve(&at(&inside, "x.txt", None), false).is_ok(), "a path inside");
        let absolute = inside.join("x.txt").display().to_string();
        assert!(resolve(&at(&inside, &absolute, None), false).is_ok(), "an absolute path inside");
        assert_eq!(kind("../out.txt", None).as_deref(), Some("outside_root"), ".. out");
        assert_eq!(kind("sub/../../out.txt", None).as_deref(), Some("outside_root"), ".. after the folder");
        let outside = dir.join("out.txt").display().to_string();
        assert_eq!(kind(&outside, None).as_deref(), Some("outside_root"), "an absolute path outside");
        assert_eq!(kind("..", None).as_deref(), Some("outside_root"), "the folder above");
        assert!(resolve(&at(&inside, &outside, Some(&dir)), false).is_ok(), "--root reaches further");
        assert_eq!(kind(&outside, Some(&inside)).as_deref(), Some("outside_root"), "a root of its own");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_is_refused_even_when_its_target_is_not_there() {
        let dir = std::env::temp_dir().join(format!("lc-links-{}", crate::platform::random_hex()));
        let inside = dir.join("root");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere")).unwrap();
        std::fs::write(dir.join("elsewhere/secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), inside.join("out")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), inside.join("dangling")).unwrap();
        std::os::unix::fs::symlink(inside.join("real.txt"), inside.join("within")).unwrap();
        let kind = |path: &str| resolve(&at(&inside, path, None), false).err().map(|e| e.kind);

        assert!(resolve(&at(&inside, "within", None), false).is_ok(), "a link within");
        assert_eq!(kind("out/secret.txt").as_deref(), Some("outside_root"), "a link out");
        assert_eq!(kind("out/new.txt").as_deref(), Some("outside_root"), "a link out, where nothing exists yet");
        assert_eq!(kind("dangling").as_deref(), Some("outside_root"), "a dangling link out");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

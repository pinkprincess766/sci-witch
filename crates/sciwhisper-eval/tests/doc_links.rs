//! Every relative Markdown link in the repository must point at something
//! that exists.
//!
//! Moving a document breaks every link that mentions it, and nothing else
//! notices: the build is green, the page on GitHub shows a dead link. This
//! test walks every `.md` file in the repository and resolves each
//! `[text](path)` relative to the file that contains it.
//!
//! What counts as a link: the inline form `[text](destination)` and images
//! `![alt](destination)`. What does not: anything inside a fenced code block
//! or an inline code span (those are examples, not links), and external or
//! anchor-only destinations (`https://…`, `mailto:…`, `#section`). The
//! `#fragment` of a file link is not checked, only that the file exists.
//!
//! The scan also refuses to pass vacuously: it must see a minimum number of
//! files and links, so a walker that silently finds nothing is a failure,
//! not a pass.

use std::path::{Path, PathBuf};

/// A repository with fewer Markdown files than this has been walked wrongly.
const MIN_MARKDOWN_FILES: usize = 30;
/// A repository with fewer relative links than this has been parsed wrongly.
const MIN_CHECKED_LINKS: usize = 100;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Directories whose contents are never documentation of this repository.
fn is_skipped_dir(name: &str) -> bool {
    matches!(name, "target" | ".git" | "node_modules")
}

fn collect_markdown(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut entries: Vec<_> = entries.map(|e| e.expect("directory entry")).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type().expect("file type");
        if kind.is_dir() {
            if !is_skipped_dir(&name) {
                collect_markdown(&path, found);
            }
        } else if kind.is_file() && name.ends_with(".md") {
            found.push(path);
        }
    }
}

/// Drops fenced code blocks (``` or ~~~), keeping line breaks so nothing
/// else shifts.
fn strip_fences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = marker.map_or(0, |m| trimmed.chars().take_while(|c| *c == m).count());
        match (fence, marker) {
            (None, Some(m)) if run >= 3 => fence = Some((m, run)),
            (Some((m, open)), Some(c))
                if c == m && run >= open && trimmed[run..].trim().is_empty() =>
            {
                fence = None;
            }
            _ => {}
        }
        if fence.is_none() && !(marker.is_some() && run >= 3) {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Drops inline code spans: a backtick run closes at the next run of the
/// same length. An unclosed run is literal text.
fn strip_inline_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] == '`' {
            i += 1;
        }
        let len = i - start;
        let mut j = i;
        let mut closed = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run_start = j;
                while j < chars.len() && chars[j] == '`' {
                    j += 1;
                }
                if j - run_start == len {
                    closed = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        match closed {
            Some(end) => i = end,
            None => out.extend(std::iter::repeat_n('`', len)),
        }
    }
    out
}

/// Destinations of inline links `[..](dest)` in `text`, after code is
/// removed. Parentheses inside a destination must balance.
fn link_destinations(markdown: &str) -> Vec<String> {
    let text = strip_inline_code(&strip_fences(markdown));
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == ']' && chars[i + 1] == '(' {
            let mut depth = 1;
            let mut j = i + 2;
            let mut dest = String::new();
            while j < chars.len() {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                dest.push(chars[j]);
                j += 1;
            }
            if depth == 0 {
                found.push(dest);
                i = j;
            }
        }
        i += 1;
    }
    found
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The filesystem part of a destination, or `None` when it is not a
/// repository-relative link (external, anchor-only, empty).
fn local_target(destination: &str) -> Option<String> {
    let mut dest = destination.trim();
    if let Some(inner) = dest.strip_prefix('<') {
        dest = inner.split('>').next().unwrap_or("");
    } else if let Some(space) = dest.find(char::is_whitespace) {
        dest = &dest[..space]; // `path "title"`
    }
    if dest.is_empty()
        || dest.starts_with('#')
        || dest.contains("://")
        || dest.starts_with("mailto:")
    {
        return None;
    }
    let path = dest.split(['#', '?']).next().unwrap_or("");
    if path.is_empty() {
        return None;
    }
    Some(percent_decode(path))
}

/// Resolves a link target the way GitHub does: relative to the file's
/// directory, or to the repository root when it starts with `/`.
fn resolve(repo_root: &Path, file: &Path, target: &str) -> PathBuf {
    match target.strip_prefix('/') {
        Some(rooted) => repo_root.join(rooted),
        None => file.parent().expect("file has a parent").join(target),
    }
}

/// Broken links in one Markdown file as `(destination)`, plus the number of
/// relative links that were checked.
fn check_file(repo_root: &Path, file: &Path) -> (Vec<String>, usize) {
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let mut broken = Vec::new();
    let mut checked = 0;
    for destination in link_destinations(&text) {
        let Some(target) = local_target(&destination) else {
            continue;
        };
        checked += 1;
        if !resolve(repo_root, file, &target).exists() {
            broken.push(destination);
        }
    }
    (broken, checked)
}

fn check_tree(repo_root: &Path) -> (Vec<String>, usize, usize) {
    let mut files = Vec::new();
    collect_markdown(repo_root, &mut files);
    let mut problems = Vec::new();
    let mut checked = 0;
    for file in &files {
        let (broken, n) = check_file(repo_root, file);
        checked += n;
        for destination in broken {
            let shown = file.strip_prefix(repo_root).unwrap_or(file);
            problems.push(format!("{}: ({destination})", shown.display()));
        }
    }
    (problems, files.len(), checked)
}

#[test]
fn every_relative_markdown_link_points_at_something_that_exists() {
    let (problems, files, checked) = check_tree(&root());
    println!("{checked} relative links checked in {files} Markdown files");
    assert!(
        files >= MIN_MARKDOWN_FILES,
        "only {files} Markdown files found (expected at least {MIN_MARKDOWN_FILES}): the walk is broken"
    );
    assert!(
        checked >= MIN_CHECKED_LINKS,
        "only {checked} relative links found (expected at least {MIN_CHECKED_LINKS}): the parser is broken"
    );
    assert!(
        problems.is_empty(),
        "{} broken relative link(s) among {checked} in {files} files:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
}

// --- the checker must fail when it should -------------------------------

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_broken_link_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("a.md"),
        "[ok](b.md) and [bad](missing.md)\n",
    );
    write(&dir.path().join("b.md"), "text\n");
    let (problems, files, checked) = check_tree(dir.path());
    assert_eq!((files, checked), (2, 2));
    assert_eq!(problems, vec!["a.md: (missing.md)".to_string()]);
}

#[test]
fn links_resolve_relative_to_the_file_not_the_root() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("docs/sub/a.md"),
        "[up](../b.md) [wrong](b.md)\n",
    );
    write(&dir.path().join("docs/b.md"), "x\n");
    let (problems, _, _) = check_tree(dir.path());
    assert_eq!(problems, vec!["docs/sub/a.md: (b.md)".to_string()]);
}

#[test]
fn a_directory_target_exists_and_a_fragment_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("a.md"),
        "[d](sub/) [f](b.md#section) [t](b.md \"title\")\n",
    );
    write(&dir.path().join("sub/x.txt"), "x\n");
    write(&dir.path().join("b.md"), "x\n");
    let (problems, _, checked) = check_tree(dir.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(checked, 3);
}

#[test]
fn a_missing_file_with_a_fragment_is_still_broken() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "[f](gone.md#section)\n");
    let (problems, _, _) = check_tree(dir.path());
    assert_eq!(problems.len(), 1);
}

#[test]
fn external_and_anchor_links_are_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("a.md"),
        "[w](https://example.org/x) [m](mailto:a@b.c) [h](#here)\n",
    );
    let (problems, _, checked) = check_tree(dir.path());
    assert!(problems.is_empty());
    assert_eq!(checked, 0);
}

#[test]
fn links_in_code_are_examples_not_links() {
    let md = "```\n[a](nope1.md)\n```\n\n~~~sh\n[b](nope2.md)\n~~~\n\n\
              `[c](nope3.md)` and ``[d](nope4.md)`` and [real](real.md)\n";
    assert_eq!(link_destinations(md), vec!["real.md".to_string()]);
}

#[test]
fn a_link_after_a_closed_fence_is_seen_again() {
    let md = "```\ncode\n```\n[x](after.md)\n";
    assert_eq!(link_destinations(md), vec!["after.md".to_string()]);
}

#[test]
fn an_unclosed_backtick_does_not_swallow_the_rest() {
    let md = "a lone ` tick then [x](still.md)\n";
    assert_eq!(link_destinations(md), vec!["still.md".to_string()]);
}

#[test]
fn a_code_span_inside_link_text_is_not_a_reason_to_skip_the_link() {
    let md = "[`code`](target.md) and ![img](pic.png)\n";
    assert_eq!(
        link_destinations(md),
        vec!["target.md".to_string(), "pic.png".to_string()]
    );
}

#[test]
fn percent_encoded_paths_are_decoded() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "[s](my%20file.md)\n");
    write(&dir.path().join("my file.md"), "x\n");
    let (problems, _, checked) = check_tree(dir.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(checked, 1);
}

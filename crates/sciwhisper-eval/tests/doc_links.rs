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
//! A second check covers repository paths written as inline code, such as
//! `` `crates/sciwhisper-eval/src/gate.rs:42` ``. Those are not links, so
//! nothing renders them as dead, yet they go stale the same way when a file
//! moves. A code span counts as a path when it starts with one of
//! `PATH_ROOTS` and is not a pattern or a template. Stale paths in files that
//! must not be edited (the changelog, the backlog, the migration map, and
//! `research/`, which is pinned by SHA-256 manifests) are listed in
//! `KNOWN_STALE_PATHS` instead, one entry per span.
//!
//! The scan also refuses to pass vacuously: it must see a minimum number of
//! files and links, so a walker that silently finds nothing is a failure,
//! not a pass.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A repository with fewer Markdown files than this has been walked wrongly.
const MIN_MARKDOWN_FILES: usize = 30;
/// A repository with fewer relative links than this has been parsed wrongly.
const MIN_CHECKED_LINKS: usize = 100;
/// A repository with fewer repository paths in code spans than this has been
/// parsed wrongly.
const MIN_CHECKED_CODE_PATHS: usize = 100;
/// Top-level directories that a repository path in a code span starts with.
const PATH_ROOTS: [&str; 7] = [
    "crates/",
    "docs/",
    "research/",
    "scripts/",
    "paper/",
    ".github/",
    "packaging/",
];

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

/// Splits inline code spans out of `text`: returns the text without them and
/// the content of each span. A backtick run closes at the next run of the
/// same length. An unclosed run is literal text.
fn split_inline_code(text: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut spans = Vec::new();
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
        let content_start = i;
        let mut j = i;
        let mut closed = None;
        while j < chars.len() {
            if chars[j] == '`' {
                let run_start = j;
                while j < chars.len() && chars[j] == '`' {
                    j += 1;
                }
                if j - run_start == len {
                    closed = Some((run_start, j));
                    break;
                }
            } else {
                j += 1;
            }
        }
        match closed {
            Some((content_end, end)) => {
                spans.push(chars[content_start..content_end].iter().collect());
                i = end;
            }
            None => out.extend(std::iter::repeat_n('`', len)),
        }
    }
    (out, spans)
}

/// Destinations of inline links `[..](dest)` in `text`, after code is
/// removed. Parentheses inside a destination must balance.
fn link_destinations(markdown: &str) -> Vec<String> {
    let text = split_inline_code(&strip_fences(markdown)).0;
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

/// A file's path relative to the repository, always with `/`: the report
/// reads the same on every platform, and Windows' `\` made the expected text
/// differ there.
fn shown(repo_root: &Path, file: &Path) -> String {
    file.strip_prefix(repo_root)
        .unwrap_or(file)
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
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
            problems.push(format!("{}: ({destination})", shown(repo_root, file)));
        }
    }
    (problems, files.len(), checked)
}

/// Whether a code span names a repository path: it starts with one of
/// `PATH_ROOTS` and is not a glob, a placeholder, a range or prose.
fn is_repo_path(span: &str) -> bool {
    PATH_ROOTS.iter().any(|prefix| span.starts_with(prefix))
        && !span.contains("...")
        && !span
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '*' | '<' | '>' | '{' | '…'))
}

/// `path:12` and `path:12:3` name a place in a file: the position is dropped
/// before the path is looked up.
fn strip_position(span: &str) -> &str {
    let mut path = span;
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((head, digits))
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
            {
                path = head;
            }
            _ => break,
        }
    }
    path
}

/// The code spans of one Markdown text that name a repository path, as
/// written.
fn repo_path_spans(markdown: &str) -> Vec<String> {
    split_inline_code(&strip_fences(markdown))
        .1
        .into_iter()
        .filter(|span| is_repo_path(span))
        .collect()
}

/// Repository paths in code spans that do not exist, as `file: `span``,
/// sorted, plus the number of files and path spans checked.
///
/// A pair `(file, span)` in `allowed` is skipped. An entry that no longer
/// misses is reported too, so the list cannot keep old entries alive.
fn check_code_paths(repo_root: &Path, allowed: &[(&str, &str)]) -> (Vec<String>, usize, usize) {
    let mut files = Vec::new();
    collect_markdown(repo_root, &mut files);
    let mut misses = BTreeSet::new();
    let mut checked = 0;
    for file in &files {
        let text =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for span in repo_path_spans(&text) {
            checked += 1;
            if !repo_root.join(strip_position(&span)).exists() {
                misses.insert((shown(repo_root, file), span));
            }
        }
    }
    let mut problems = Vec::new();
    let mut used = vec![false; allowed.len()];
    for (file, span) in &misses {
        match allowed.iter().position(|(f, s)| f == file && s == span) {
            Some(i) => used[i] = true,
            None => problems.push(format!("{file}: `{span}`")),
        }
    }
    for (i, (file, span)) in allowed.iter().enumerate() {
        if !used[i] {
            problems.push(format!(
                "allow-list entry no longer misses: {file}: `{span}`"
            ));
        }
    }
    problems.sort();
    (problems, files.len(), checked)
}

/// Whether a scan that found `checked` repository paths in code spans saw
/// enough of them to have walked the repository.
fn saw_enough_code_paths(checked: usize) -> bool {
    checked >= MIN_CHECKED_CODE_PATHS
}

/// Stale paths that stay as they are, as `(file, span)`. Each entry says why
/// it is not fixed. Only for files that are not edited on purpose: the
/// history in CHANGELOG.md, the backlog, the migration map in
/// docs/development/README.md, and text that states a file is absent.
const KNOWN_STALE_PATHS: &[(&str, &str)] = &[
    // CHANGELOG: release notes name the file as it was at that release.
    ("CHANGELOG.md", ".github/workflows/release.yml"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "docs/images/"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "docs/process/AUTO_UPDATE_RU.md"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "docs/user/USAGE_RU.md"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "packaging/branding"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "packaging/linux"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "packaging/macos"),
    // CHANGELOG: removed with the voice app (tag app-0.5-final).
    ("CHANGELOG.md", "packaging/windows"),
    // CHANGELOG: deleted from the Python runtime, which is not in the product.
    ("CHANGELOG.md", "scripts/whisperd.py"),
    // README: states that this file is absent from the tree.
    ("README.md", "research/results/voice-v1.json"),
    // README.ru: states that this file is absent from the tree.
    ("README.ru.md", "research/results/voice-v1.json"),
    // BACKLOG: the planned report of an open item; it was never created.
    (
        "docs/process/BACKLOG_RU.md",
        "research/results/pcfg-v1.json",
    ),
    // Plan: a removed file, named as removed.
    (
        "docs/research/sci-witch-plan.md",
        "docs/images/si-witch-recording.gif",
    ),
    // Plan: a removed directory, named as removed.
    ("docs/research/sci-witch-plan.md", "docs/images"),
    // Plan: a planned script that was never created.
    ("docs/research/sci-witch-plan.md", "scripts/reproduce.sh"),
    // Migration map: the old path, removed with the voice app.
    (
        "docs/development/README.md",
        ".github/workflows/release.yml",
    ),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "crates/sciwhisper-shell/"),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "crates/sciwhisper-update/"),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/ARCHITECTURE_RU.md",
    ),
    // Migration map: the old path, removed with the voice app.
    (
        "docs/development/README.md",
        "docs/development/AUTO_UPDATE_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/BALANCE_KERNEL_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/CANDIDATE_LATTICE_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/CHEMISTRY_NOMENCLATURE_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/COMPILER_CONTRACT_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/FORMAL_GUARANTEES_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/GRAMMAR_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/IUPAC_SOURCES.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/MATHEMATICS_RU.md",
    ),
    // Migration map: the old path, moved to docs/research/.
    (
        "docs/development/README.md",
        "docs/development/ML_LAB_RU.md",
    ),
    // Migration map: the old path, moved to docs/research/.
    (
        "docs/development/README.md",
        "docs/development/ML_RESEARCH_RU.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/NATURAL_DICTATION_RU.md",
    ),
    // Migration map: the old path, moved to docs/process/.
    (
        "docs/development/README.md",
        "docs/development/PAIR_WORKFLOW_RU.md",
    ),
    // Migration map: the old path, removed with the voice app.
    (
        "docs/development/README.md",
        "docs/development/RELEASE_CHECKLIST.md",
    ),
    // Migration map: the old path, moved to docs/compiler/.
    (
        "docs/development/README.md",
        "docs/development/SPECIFICATION_RU.md",
    ),
    // Migration map: the old path, moved to docs/decisions/.
    ("docs/development/README.md", "docs/development/decisions/"),
    // Migration map: the old path, moved to docs/research/.
    (
        "docs/development/README.md",
        "docs/development/sci-witch-plan.md",
    ),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "docs/images/"),
    // Migration map: the old path, removed with the voice app.
    (
        "docs/development/README.md",
        "docs/process/AUTO_UPDATE_RU.md",
    ),
    // Migration map: the old path, removed with the voice app.
    (
        "docs/development/README.md",
        "docs/process/RELEASE_CHECKLIST.md",
    ),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "docs/user/USAGE_RU.md"),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "packaging/branding/"),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "packaging/linux/"),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "packaging/macos/"),
    // Migration map: the old path, removed with the voice app.
    ("docs/development/README.md", "packaging/windows/"),
];

#[test]
fn every_repository_path_in_code_spans_exists() {
    let (problems, files, checked) = check_code_paths(&root(), KNOWN_STALE_PATHS);
    println!("{checked} repository paths in code spans checked in {files} Markdown files");
    assert!(
        saw_enough_code_paths(checked),
        "only {checked} repository paths in code spans found (expected at least {MIN_CHECKED_CODE_PATHS}): the scan is broken"
    );
    assert!(
        problems.is_empty(),
        "{} stale repository path(s) in code spans among {checked} in {files} files:\n  {}",
        problems.len(),
        problems.join("\n  ")
    );
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

// --- repository paths in code spans -------------------------------------

#[test]
fn a_missing_code_path_is_reported_and_the_list_is_sorted() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("b.md"),
        "`docs/zeta.md` and `crates/alpha.rs`\n",
    );
    write(&dir.path().join("a.md"), "`docs/nope.md`\n");
    write(&dir.path().join("docs/real.md"), "x\n");
    let (problems, files, checked) = check_code_paths(dir.path(), &[]);
    assert_eq!((files, checked), (3, 3));
    assert_eq!(
        problems,
        vec![
            "a.md: `docs/nope.md`".to_string(),
            "b.md: `crates/alpha.rs`".to_string(),
            "b.md: `docs/zeta.md`".to_string(),
        ]
    );
}

#[test]
fn an_existing_file_or_directory_passes_with_or_without_a_position() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("a.md"),
        "`crates/x/src/lib.rs` `crates/x/src/lib.rs:42` `crates/x/src/lib.rs:42:7` `docs/` `docs/sub`\n",
    );
    write(&dir.path().join("crates/x/src/lib.rs"), "x\n");
    write(&dir.path().join("docs/sub/y.md"), "x\n");
    let (problems, _, checked) = check_code_paths(dir.path(), &[]);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(checked, 5);
}

#[test]
fn a_position_is_dropped_before_the_lookup_but_a_missing_file_stays_missing() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "`docs/gone.md:3:9`\n");
    let (problems, _, _) = check_code_paths(dir.path(), &[]);
    assert_eq!(problems, vec!["a.md: `docs/gone.md:3:9`".to_string()]);
}

#[test]
fn globs_templates_ranges_prose_and_other_roots_are_not_paths() {
    let md = "`research/results/*.json` `research/data/<corpus_id>.jsonl` \
              `research/{a,b}.json` `docs/…` `docs/...` `docs/a b.md` \
              `src/main.rs` `Cargo.toml` `crates` `my/docs/a.md`\n";
    assert_eq!(repo_path_spans(md), Vec::<String>::new());
}

#[test]
fn code_spans_in_fences_are_not_checked_and_double_ticks_are_read() {
    let md = "```\n`docs/fenced.md`\n```\n``docs/double.md:4`` and `docs/single.md`\n";
    assert_eq!(
        repo_path_spans(md),
        vec!["docs/double.md:4".to_string(), "docs/single.md".to_string()]
    );
}

#[test]
fn an_allowed_miss_is_skipped_and_its_neighbour_is_not() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "`docs/old.md` `docs/other.md`\n");
    let (problems, _, checked) = check_code_paths(dir.path(), &[("a.md", "docs/old.md")]);
    assert_eq!(problems, vec!["a.md: `docs/other.md`".to_string()]);
    assert_eq!(checked, 2);
}

#[test]
fn an_allow_list_entry_that_no_longer_misses_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a.md"), "`docs/here.md`\n");
    write(&dir.path().join("docs/here.md"), "x\n");
    let allowed = [("a.md", "docs/here.md"), ("a.md", "docs/gone.md")];
    let (problems, _, _) = check_code_paths(dir.path(), &allowed);
    assert_eq!(
        problems,
        vec![
            "allow-list entry no longer misses: a.md: `docs/gone.md`".to_string(),
            "allow-list entry no longer misses: a.md: `docs/here.md`".to_string(),
        ]
    );
}

#[test]
fn the_code_path_floor_is_exactly_the_minimum() {
    assert!(!saw_enough_code_paths(MIN_CHECKED_CODE_PATHS - 1));
    assert!(saw_enough_code_paths(MIN_CHECKED_CODE_PATHS));
}

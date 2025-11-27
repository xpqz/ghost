use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use pulldown_cmark::{Event, Parser, Tag};
use scraper::{Html, Selector};

#[derive(Debug, Deserialize)]
pub struct MkDocsConfig {
    pub nav: Vec<NavItem>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum NavItem {
    Page(HashMap<String, String>),
    Section(HashMap<String, Vec<NavItem>>),
    PlainPath(String),
}

#[derive(Debug)]
pub struct AuditResult {
    pub nav_missing: Vec<PathBuf>,
    pub ghost: Vec<PathBuf>,
    pub help_missing: Vec<PathBuf>,
}

pub fn audit(mkdocs_yaml: &Path, help_urls: &Path) -> Result<AuditResult, Box<dyn Error>> {
    let contents = fs::read_to_string(mkdocs_yaml)?;
    let config: MkDocsConfig = serde_yaml::from_str(&contents)?;
    let mut pages = HashSet::<PathBuf>::new();
    let parent = mkdocs_yaml.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "mkdocs file must reside within a directory",
        )
    })?;
    collect_pages(&config.nav, &mut pages, parent)?;
    let nav_missing = missing_files(&pages);
    let mut markdown_roots = Vec::new();
    markdown_roots.extend(include_roots(&config.nav, parent));
    let files = find_markdown(markdown_roots)?;
    let mut ghost = orphans(&pages, &files); // markdown files in the file system not referenced by nav

    let help_files = extract_help_urls(help_urls, parent);
    let help_missing = missing_files(&help_files);

    ghost.retain(|x| !help_files.contains(x));

    let links: Vec<String> = normalise_links(
        files
            .iter()
            .map(|p| fs::read_to_string(p).map(|c| extract_links(&c)))
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .flatten(),
    );

    Ok(AuditResult {
        nav_missing,
        ghost,
        help_missing,
    })
}

pub fn extract_links(markdown: &str) -> Vec<String> {
    let mut links = Vec::new();
    let parser = Parser::new(markdown);
    let link_selector = Selector::parse("a[href]").unwrap();

    for event in parser {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                links.push(dest_url.into_string());
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let fragment = Html::parse_fragment(&html);
                for el in fragment.select(&link_selector) {
                    if let Some(href) = el.value().attr("href") {
                        links.push(href.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    links
}

pub fn normalise_links<I>(links: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    links
        .into_iter()
        .filter_map(|link| {
            // drop page-internal anchors first
            let mut link = link.split('#').next().unwrap_or("").trim().to_string();
            if link.is_empty() {
                return None;
            }

            // skip externals and mailto
            if link.starts_with("http") || link.starts_with("mailto:") {
                return None;
            }

            // trailing slash → strip and add .md
            if link.ends_with('/') {
                link = link.trim_end_matches('/').to_string();
                if link.is_empty() {
                    return None;
                }
                link.push_str(".md");
                return Some(link);
            }

            let path = Path::new(&link);
            match path.extension() {
                Some(ext) if ext == "md" => Some(link), // already markdown
                Some(_) => None,                        // non-markdown => drop
                None => {
                    // add .md when no extension
                    let mut with_ext = link;
                    with_ext.push_str(".md");
                    Some(with_ext)
                }
            }
        })
        .collect()
}

pub fn collect_pages(
    items: &[NavItem],
    pages: &mut HashSet<PathBuf>,
    prefix: &Path,
) -> Result<(), Box<dyn Error>> {
    for item in items {
        match item {
            NavItem::Page(map) => {
                for path in map.values() {
                    if let Some(include_path) = parse_include_target(path) {
                        collect_include(include_path, pages, prefix)?;
                    } else {
                        let full_path = prefix.join("docs").join(path);
                        let normalised = full_path.components().collect::<PathBuf>(); // removes "./"
                        pages.insert(normalised);
                    }
                }
            }
            NavItem::Section(map) => {
                for children in map.values() {
                    collect_pages(children, pages, prefix)?;
                }
            }
            NavItem::PlainPath(path) => {
                let full_path = prefix.join("docs").join(path);
                let normalised = full_path.components().collect::<PathBuf>(); // removes "./"
                pages.insert(normalised);
            }
        }
    }

    Ok(())
}

fn include_roots(items: &[NavItem], prefix: &Path) -> Vec<PathBuf> {
    let mut roots = HashSet::new();
    for item in items {
        match item {
            NavItem::Page(map) => {
                for path in map.values() {
                    if let Some(include_path) = parse_include_target(path) {
                        let include_dir = prefix.join(include_path);
                        if let Some(parent) = include_dir.parent() {
                            roots.insert(parent.components().collect::<PathBuf>());
                        }
                    }
                }
            }
            NavItem::Section(map) => {
                for children in map.values() {
                    roots.extend(include_roots(children, prefix));
                }
            }
            NavItem::PlainPath(_) => {}
        }
    }
    roots.into_iter().collect()
}

fn parse_include_target(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("!include")
        .map(|target| target.trim().trim_matches(|c| c == '"' || c == '\''))
}

fn collect_include(
    include_path: &str,
    pages: &mut HashSet<PathBuf>,
    prefix: &Path,
) -> Result<(), Box<dyn Error>> {
    let include_file = prefix.join(include_path);
    let include_contents = fs::read_to_string(&include_file)?;
    let include_config: MkDocsConfig = serde_yaml::from_str(&include_contents)?;
    let include_parent = include_file.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "included mkdocs file must reside within a directory",
        )
    })?;
    collect_pages(&include_config.nav, pages, include_parent)?;
    Ok(())
}

fn strip_c_comments(content: &str) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();
    let mut in_block_comment = false;

    while let Some(ch) = chars.next() {
        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next(); // consume the '/'
                in_block_comment = false;
            }
        } else if ch == '/' {
            match chars.peek() {
                Some(&'/') => {
                    // Line comment - skip until end of line
                    while let Some(&next_ch) = chars.peek() {
                        chars.next();
                        if next_ch == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                }
                Some(&'*') => {
                    // Block comment
                    chars.next(); // consume the '*'
                    in_block_comment = true;
                }
                _ => {
                    result.push(ch);
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

pub fn extract_help_urls<P1, P2>(path: P1, doc_root: P2) -> Vec<PathBuf>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let raw_content = fs::read_to_string(path).expect("failed to read file");
    let content = strip_c_comments(&raw_content);

    let define = Regex::new(r#"#define\s+(\w+)\s+"([^"]+)""#).unwrap();
    let macros: HashMap<String, String> = define
        .captures_iter(&content)
        .map(|cap| {
            // Store macro values without /docs/ injection - we'll inject later
            let name = cap.get(1).unwrap().as_str().to_string();
            let path = cap.get(2).unwrap().as_str().trim().to_string();
            (name, path)
        })
        .collect();

    // Match HELP_URL("first_arg", second_arg) where:
    // - first_arg is a quoted string (may contain any character including comma)
    // - second_arg can be a quoted string, macro, or concatenation like MACRO"/suffix"
    // We need to match the quoted first argument, then capture everything after the comma
    let url_re = Regex::new(r#"HELP_URL\s*\("([^"]|\\")*"\s*,\s*([^)]+)\)"#).unwrap();

    url_re
        .captures_iter(&content)
        .filter_map(|cap| {
            let raw = cap.get(2).unwrap().as_str().trim();
            let expanded = expand_url(raw, &macros);
            let with_docs = inject_docs(&expanded);
            let relative_path = with_docs + ".md";
            let absolute_path = doc_root.as_ref().join(relative_path);
            Some(absolute_path)
        })
        .collect()
}

fn expand_url(raw: &str, macros: &HashMap<String, String>) -> String {
    let mut result = String::new();
    for part in raw.split('"') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(expanded) = macros.get(trimmed) {
            result.push_str(expanded);
        } else {
            result.push_str(trimmed);
        }
    }
    result
}

fn inject_docs(path: &str) -> String {
    // Inject /docs/ after the first path component
    // e.g., "language-reference-guide/symbols/comma" -> "language-reference-guide/docs/symbols/comma"
    let mut comps = Path::new(path).components();
    let mut with_docs = PathBuf::new();
    if let Some(first) = comps.next() {
        with_docs.push(first.as_os_str());
        with_docs.push("docs");
        for c in comps {
            with_docs.push(c.as_os_str());
        }
    } else {
        with_docs.push("docs");
    }
    with_docs.to_string_lossy().into_owned()
}

pub fn missing_files<'a, I>(pages: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = &'a PathBuf>,
{
    pages
        .into_iter()
        .filter(|p| !p.is_file())
        .cloned()
        .collect()
}

fn find_markdown<P, I>(roots: I) -> io::Result<Vec<PathBuf>>
where
    P: AsRef<Path>,
    I: IntoIterator<Item = P>,
{
    let mut out = Vec::new();

    for root in roots {
        let mut paths = WalkDir::new(root.as_ref())
            .into_iter()
            .map(|res| {
                let entry = res.map_err(walkdir_error)?;
                let path = entry.path();
                if entry.file_type().is_file() && path.extension().is_some_and(|ext| ext == "md") {
                    let normalised = path.components().collect::<PathBuf>();
                    Ok(Some(normalised))
                } else {
                    Ok(None)
                }
            })
            .filter_map(Result::transpose) // flatten Option<Result<_>> to Result<Option<_>>
            .collect::<io::Result<Vec<_>>>()?; // error-propagating collect
        out.append(&mut paths);
    }

    Ok(out)
}

fn walkdir_error(err: walkdir::Error) -> io::Error {
    let msg = err.to_string();
    err.into_io_error()
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, msg))
}

pub fn orphans(nav: &HashSet<PathBuf>, files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|p| !nav.contains(*p))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_pages_with_page() {
        let nav = vec![NavItem::Page({
            let mut map = HashMap::new();
            map.insert("Title".to_string(), "path/to/file.md".to_string());
            map
        })];

        let mut pages = HashSet::new();
        let prefix = Path::new("/tmp/docs");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&PathBuf::from("/tmp/docs/docs/path/to/file.md")));
    }

    #[test]
    fn test_collect_pages_with_plain_path() {
        let nav = vec![NavItem::PlainPath("example.md".to_string())];

        let mut pages = HashSet::new();
        let prefix = Path::new("/tmp/docs");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&PathBuf::from("/tmp/docs/docs/example.md")));
    }

    #[test]
    fn test_collect_pages_with_section() {
        let nav = vec![NavItem::Section({
            let mut map = HashMap::new();
            map.insert(
                "Section".to_string(),
                vec![NavItem::Page({
                    let mut inner = HashMap::new();
                    inner.insert("Page1".to_string(), "page1.md".to_string());
                    inner
                })],
            );
            map
        })];

        let mut pages = HashSet::new();
        let prefix = Path::new("/tmp/docs");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&PathBuf::from("/tmp/docs/docs/page1.md")));
    }

    #[test]
    fn test_collect_pages_nested_sections() {
        let nav = vec![NavItem::Section({
            let mut map = HashMap::new();
            map.insert(
                "Outer".to_string(),
                vec![NavItem::Section({
                    let mut inner_map = HashMap::new();
                    inner_map.insert(
                        "Inner".to_string(),
                        vec![NavItem::PlainPath("nested/file.md".to_string())],
                    );
                    inner_map
                })],
            );
            map
        })];

        let mut pages = HashSet::new();
        let prefix = Path::new("/base");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&PathBuf::from("/base/docs/nested/file.md")));
    }

    #[test]
    fn test_collect_pages_multiple_items() {
        let nav = vec![
            NavItem::PlainPath("first.md".to_string()),
            NavItem::Page({
                let mut map = HashMap::new();
                map.insert("Second".to_string(), "second.md".to_string());
                map
            }),
            NavItem::Section({
                let mut map = HashMap::new();
                map.insert(
                    "Section".to_string(),
                    vec![NavItem::PlainPath("third.md".to_string())],
                );
                map
            }),
        ];

        let mut pages = HashSet::new();
        let prefix = Path::new("/root");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 3);
        assert!(pages.contains(&PathBuf::from("/root/docs/first.md")));
        assert!(pages.contains(&PathBuf::from("/root/docs/second.md")));
        assert!(pages.contains(&PathBuf::from("/root/docs/third.md")));
    }

    #[test]
    fn test_collect_pages_empty_nav() {
        let nav = vec![];
        let mut pages = HashSet::new();
        let prefix = Path::new("/tmp");

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 0);
    }

    #[test]
    fn test_collect_pages_with_include() {
        let temp_dir = tempfile::tempdir().unwrap();
        let prefix = temp_dir.path();

        let include_dir = prefix.join("release-notes");
        fs::create_dir_all(include_dir.join("docs")).unwrap();
        let include_yaml = r#"
nav:
  - Child Page: child.md
"#;
        fs::write(include_dir.join("mkdocs.yml"), include_yaml).unwrap();

        let nav = vec![NavItem::Page({
            let mut map = HashMap::new();
            map.insert(
                "Release Notes".to_string(),
                "!include ./release-notes/mkdocs.yml".to_string(),
            );
            map
        })];

        let mut pages = HashSet::new();

        collect_pages(&nav, &mut pages, prefix).unwrap();

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&include_dir.join("docs").join("child.md")));
    }

    #[test]
    fn test_missing_files_all_exist() {
        let mut pages = HashSet::new();
        pages.insert(PathBuf::from("src/lib.rs"));

        let missing = missing_files(&pages);

        assert!(missing.is_empty());
    }

    #[test]
    fn test_missing_files_detects_missing() {
        let mut pages = HashSet::new();
        pages.insert(PathBuf::from("/nonexistent/file.md"));

        let missing = missing_files(&pages);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0], PathBuf::from("/nonexistent/file.md"));
    }

    #[test]
    fn test_orphans_no_orphans() {
        let mut nav = HashSet::new();
        nav.insert(PathBuf::from("/docs/file1.md"));
        nav.insert(PathBuf::from("/docs/file2.md"));

        let files = vec![
            PathBuf::from("/docs/file1.md"),
            PathBuf::from("/docs/file2.md"),
        ];

        let orphan_files = orphans(&nav, &files);

        assert_eq!(orphan_files.len(), 0);
    }

    #[test]
    fn test_orphans_some_orphans() {
        let mut nav = HashSet::new();
        nav.insert(PathBuf::from("/docs/included.md"));

        let files = vec![
            PathBuf::from("/docs/included.md"),
            PathBuf::from("/docs/orphan1.md"),
            PathBuf::from("/docs/orphan2.md"),
        ];

        let orphan_files = orphans(&nav, &files);

        assert_eq!(orphan_files.len(), 2);
        assert!(orphan_files.contains(&PathBuf::from("/docs/orphan1.md")));
        assert!(orphan_files.contains(&PathBuf::from("/docs/orphan2.md")));
    }

    #[test]
    fn test_orphans_all_orphans() {
        let nav = HashSet::new();

        let files = vec![
            PathBuf::from("/docs/orphan1.md"),
            PathBuf::from("/docs/orphan2.md"),
        ];

        let orphan_files = orphans(&nav, &files);

        assert_eq!(orphan_files.len(), 2);
    }

    #[test]
    fn test_orphans_empty_files() {
        let mut nav = HashSet::new();
        nav.insert(PathBuf::from("/docs/file.md"));

        let files = vec![];

        let orphan_files = orphans(&nav, &files);

        assert!(orphan_files.is_empty());
    }

    #[test]
    fn test_strip_c_comments_line_comment() {
        let input = "line1\n// this is a comment\nline2";
        let result = strip_c_comments(input);
        assert_eq!(result, "line1\n\nline2");
    }

    #[test]
    fn test_strip_c_comments_block_comment() {
        let input = "line1\n/* block comment */line2";
        let result = strip_c_comments(input);
        assert_eq!(result, "line1\nline2");
    }

    #[test]
    fn test_expand_url_simple_string() {
        let macros = HashMap::new();
        let result = expand_url("path/to/file", &macros);
        assert_eq!(result, "path/to/file");
    }

    #[test]
    fn test_expand_url_with_macro() {
        let mut macros = HashMap::new();
        macros.insert("SY".to_string(), "language-reference-guide/symbols".to_string());
        let result = expand_url("SY\"/comma\"", &macros);
        assert_eq!(result, "language-reference-guide/symbols/comma");
    }

    #[test]
    fn test_inject_docs() {
        let result = inject_docs("language-reference-guide/symbols/comma");
        assert_eq!(result, "language-reference-guide/docs/symbols/comma");
    }

    #[test]
    fn test_inject_docs_single_component() {
        let result = inject_docs("file");
        assert_eq!(result, "file/docs");
    }

    #[test]
    fn test_extract_help_urls_ignores_comments() {
        use tempfile::NamedTempFile;
        use std::io::Write;

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
// This is a comment with HELP_URL("test", "fake/path")
#define SY "language-reference-guide/symbols"
HELP_URL(",", SY"/comma")
"#
        )
        .unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let doc_root = temp_dir.path();

        let result = extract_help_urls(temp_file.path(), doc_root);

        // Should have 1 URL (the comma one), not 2 (comment should be ignored)
        assert_eq!(result.len(), 1);
        assert!(result[0].to_string_lossy().contains("language-reference-guide/docs/symbols/comma.md"));
    }

    #[test]
    fn test_extract_help_urls_expands_macros() {
        use tempfile::NamedTempFile;
        use std::io::Write;

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
#define SY "language-reference-guide/symbols"
HELP_URL(",", SY"/comma")
"#
        )
        .unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let doc_root = temp_dir.path();

        let result = extract_help_urls(temp_file.path(), doc_root);

        assert_eq!(result.len(), 1);
        let path_str = result[0].to_string_lossy();
        assert!(path_str.contains("language-reference-guide/docs/symbols/comma.md"));
    }

    #[test]
    fn test_extract_help_urls_injects_docs() {
        use tempfile::NamedTempFile;
        use std::io::Write;

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            r#"
HELP_URL(":if", "programming-reference-guide/defined-functions-and-operators/traditional-functions-and-operators/control-structures/if")
"#
        )
        .unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let doc_root = temp_dir.path();

        let result = extract_help_urls(temp_file.path(), doc_root);

        assert_eq!(result.len(), 1);
        let path_str = result[0].to_string_lossy();
        assert!(path_str.contains("/docs/"));
        assert!(path_str.contains("programming-reference-guide/docs/defined-functions-and-operators"));
    }
}

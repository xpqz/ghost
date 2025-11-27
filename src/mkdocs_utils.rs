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
    pub broken_links: Vec<BrokenLink>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BrokenLink {
    pub from: PathBuf,
    pub link: String,
}

#[derive(Debug, Default)]
pub struct LinkMaps {
    pub url_to_src: HashMap<String, PathBuf>,
    pub src_to_url: HashMap<PathBuf, String>,
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
    // parent dir MUST NOT BE INCLUDED in markdown_roots!
    markdown_roots.extend(include_roots(&config.nav, parent));
    let files = find_markdown(markdown_roots)?;
    let files_set: HashSet<PathBuf> = files.iter().cloned().collect();
    let mut ghost = orphans(&pages, &files); // markdown files in the file system not referenced by nav

    // For link analysis, read only nav-listed pages that actually exist to avoid scanning everything under parent
    let pages_existing: Vec<PathBuf> = pages.iter().filter(|p| p.is_file()).cloned().collect();
    let link_files_set: HashSet<PathBuf> = files_set
        .union(&pages_existing.iter().cloned().collect())
        .cloned()
        .collect();
    let file_contents: Vec<(PathBuf, String)> = pages_existing
        .iter()
        .map(|p| fs::read_to_string(p).map(|c| (p.clone(), c)))
        .collect::<io::Result<_>>()?;

    let link_maps = build_link_maps(&config.nav, parent)?;

    let help_files = extract_help_urls(help_urls, parent);
    let help_missing = missing_files(&help_files);

    ghost.retain(|x| !help_files.contains(x));

    let (referenced, broken_links) =
        analyse_links(&file_contents, &link_files_set, parent, &link_maps)?;

    ghost.retain(|p| !referenced.contains(p));

    Ok(AuditResult {
        nav_missing,
        ghost,
        help_missing,
        broken_links,
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

// Complex. Map the "virtual" hierarchy as defined by the nav onto the file system so that
// we can check links for validity.
pub fn build_link_maps(nav: &[NavItem], mkdocs_dir: &Path) -> Result<LinkMaps, Box<dyn Error>> {
    let mut maps = LinkMaps::default();
    build_link_maps_inner(
        nav,
        mkdocs_dir,
        mkdocs_dir,
        Path::new(""),
        &mut maps.url_to_src,
        &mut maps.src_to_url,
    )?;
    Ok(maps)
}

fn build_link_maps_inner(
    nav: &[NavItem],
    mkdocs_dir: &Path,
    site_root: &Path,
    url_prefix: &Path,
    url_to_src: &mut HashMap<String, PathBuf>,
    src_to_url: &mut HashMap<PathBuf, String>,
) -> Result<(), Box<dyn Error>> {
    for item in nav {
        match item {
            NavItem::Page(map) => {
                for path in map.values() {
                    if let Some(include_path) = parse_include_target(path) {
                        let include_file = mkdocs_dir.join(include_path);
                        let include_contents = fs::read_to_string(&include_file)?;
                        let include_config: MkDocsConfig = serde_yaml::from_str(&include_contents)?;
                        let include_parent = include_file
                            .parent()
                            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "include has no parent"))?
                            .components()
                            .collect::<PathBuf>();
                        let mut child_prefix = url_prefix.to_path_buf();
                        if let Ok(rel) = include_parent.strip_prefix(site_root) {
                            child_prefix = child_prefix.join(rel);
                        }
                        build_link_maps_inner(
                            &include_config.nav,
                            &include_parent,
                            site_root,
                            &child_prefix,
                            url_to_src,
                            src_to_url,
                        )?;
                    } else {
                        insert_mapping(path, mkdocs_dir, url_prefix, url_to_src, src_to_url);
                    }
                }
            }
            NavItem::Section(map) => {
                for children in map.values() {
                    build_link_maps_inner(
                        children,
                        mkdocs_dir,
                        site_root,
                        url_prefix,
                        url_to_src,
                        src_to_url,
                    )?;
                }
            }
            NavItem::PlainPath(path) => {
                insert_mapping(path, mkdocs_dir, url_prefix, url_to_src, src_to_url);
            }
        }
    }

    Ok(())
}

fn insert_mapping(
    nav_path: &str,
    mkdocs_dir: &Path,
    url_prefix: &Path,
    url_to_src: &mut HashMap<String, PathBuf>,
    src_to_url: &mut HashMap<PathBuf, String>,
) {
    let fs_path = mkdocs_dir
        .join("docs")
        .join(nav_path)
        .components()
        .collect::<PathBuf>();
    let mut rendered = url_prefix.to_path_buf();
    rendered.push(Path::new(nav_path));
    let rendered = rendered.with_extension("");
    let rendered = normalise_url(&rendered);
    url_to_src.entry(rendered.clone()).or_insert(fs_path.clone());
    src_to_url.entry(fs_path).or_insert(rendered);
}

fn normalise_url(path: &Path) -> String {
    let mut parts = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                parts.pop();
            }
            _ => parts.push(comp.as_os_str().to_string_lossy().into_owned()),
        }
    }
    parts.join("/")
}

pub fn resolve_link(from_src: &Path, link: &str, maps: &LinkMaps) -> Option<PathBuf> {
    let rendered = rendered_url_for_link(from_src, link, maps)?;
    lookup_url(&rendered, &maps.url_to_src)
}

fn rendered_url_for_link(from_src: &Path, link: &str, maps: &LinkMaps) -> Option<String> {
    let from_url = maps.src_to_url.get(from_src)?;
    let base = Path::new(from_url);
    let target = link.trim_start_matches('/');
    let mut joined = if link.starts_with('/') {
        PathBuf::from(target)
    } else {
        let parent = base.parent().unwrap_or(Path::new(""));
        parent.join(target)
    };
    joined = joined.with_extension("");
    Some(normalise_url(&joined))
}

fn lookup_url(rendered: &str, url_to_src: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    if let Some(p) = url_to_src.get(rendered) {
        return Some(p.clone());
    }
    let mut alt = rendered.trim_end_matches('/').to_string();
    if let Some(p) = url_to_src.get(&alt) {
        return Some(p.clone());
    }
    alt.push_str("/index");
    url_to_src.get(&alt).cloned()
}

fn analyse_links(
    files: &[(PathBuf, String)],
    files_set: &HashSet<PathBuf>,
    mkdocs_dir: &Path,
    link_maps: &LinkMaps,
) -> io::Result<(HashSet<PathBuf>, Vec<BrokenLink>)> {
    let mut referenced = HashSet::new();
    let mut broken_links = Vec::new();

    for (src, content) in files {
        let links = normalise_links(extract_links(content));
        #[cfg(test)]
        eprintln!("analysing {} links for {}", links.len(), src.display());
        for link in links {
            // 1) Try nav-based resolution
            if let Some(target) = resolve_link(src, &link, link_maps) {
                if target.is_file() || files_set.contains(&target) {
                    #[cfg(test)]
                    eprintln!("resolved via nav: {} -> {}", link, target.display());
                    referenced.insert(target);
                    continue;
                }
            }

            // 2) Try URL-derived filesystem guess under mkdocs_dir/docs
            if let Some(rendered) = rendered_url_for_link(src, &link, link_maps) {
                let fs_guess = mkdocs_dir
                    .join("docs")
                    .join(&rendered)
                    .with_extension("md")
                    .components()
                    .collect::<PathBuf>();
                if fs_guess.is_file() || files_set.contains(&fs_guess) {
                    #[cfg(test)]
                    eprintln!("resolved via fs guess: {} -> {}", link, fs_guess.display());
                    referenced.insert(fs_guess);
                    continue;
                }
            }

            // 3) Try filesystem-relative resolution
            let fs_target = if link.starts_with('/') {
                mkdocs_dir.join("docs").join(link.trim_start_matches('/'))
            } else {
                src.parent()
                    .unwrap_or(mkdocs_dir)
                    .join(&link)
                    .components()
                    .collect::<PathBuf>()
            };

        if fs_target.is_file() || files_set.contains(&fs_target) {
            #[cfg(test)]
            eprintln!("resolved via fs target: {} -> {}", link, fs_target.display());
            referenced.insert(fs_target);
            continue;
        }

        // 4) Unresolved
        #[cfg(test)]
        eprintln!("broken: {} -> {}", src.display(), link);
        broken_links.push(BrokenLink {
            from: src.clone(),
            link: link.clone(),
        });
    }
    }

    #[cfg(test)]
    eprintln!("returning broken_links len {}", broken_links.len());
    Ok((referenced, broken_links))
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
    fn test_build_link_maps_with_include_and_resolution() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let root_docs = root.join("docs");
        fs::create_dir_all(root_docs.join("dir")).unwrap();
        fs::write(root_docs.join("a.md"), "# A").unwrap();
        fs::write(root_docs.join("dir").join("b.md"), "# B").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let inc_dir = root.join("release-notes");
        fs::create_dir_all(inc_dir.join("docs")).unwrap();
        fs::write(inc_dir.join("docs").join("child.md"), "# Child").unwrap();

        let inc_mkdocs = r#"
nav:
  - Child: child.md
"#;
        fs::write(inc_dir.join("mkdocs.yml"), inc_mkdocs).unwrap();

        let nav = vec![
            NavItem::Page({
                let mut m = HashMap::new();
                m.insert("A".to_string(), "a.md".to_string());
                m
            }),
            NavItem::Page({
                let mut m = HashMap::new();
                m.insert("B".to_string(), "dir/b.md".to_string());
                m
            }),
            NavItem::Page({
                let mut m = HashMap::new();
                m.insert("Include".to_string(), "!include ./release-notes/mkdocs.yml".to_string());
                m
            }),
        ];

        let maps = build_link_maps(&nav, root).unwrap();
        let keys: Vec<String> = maps.url_to_src.keys().cloned().collect();
        assert!(
            keys.contains(&"release-notes/child".to_string()),
            "keys: {:?}",
            keys
        );
        assert_eq!(
            maps.url_to_src.get("a").unwrap(),
            &root.join("docs").join("a.md")
        );
        assert_eq!(
            maps.url_to_src.get("release-notes/child").unwrap(),
            &inc_dir.join("docs").join("child.md")
        );

        let from_src = root_docs.join("dir").join("b.md");
        let target = resolve_link(&from_src, "../a.md", &maps).unwrap();
        assert_eq!(target, root_docs.join("a.md"));

        let target2 = resolve_link(&from_src, "/release-notes/child.md", &maps).unwrap();
        assert_eq!(target2, inc_dir.join("docs").join("child.md"));
    }

    #[test]
    fn test_ghost_removed_when_linked() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("a.md"), "[Link](orphan)").unwrap();
        fs::write(docs.join("orphan.md"), "# Orphan").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - A: a.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(!result.ghost.contains(&docs.join("orphan.md")));
        assert!(result.broken_links.is_empty());
    }

    #[test]
    fn test_broken_link_reported() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("a.md"), "[Missing](missing)").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - A: a.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let extracted = extract_links(&fs::read_to_string(docs.join("a.md")).unwrap());
        assert_eq!(extracted, vec!["missing"]);

        let files = find_markdown(vec![root]).unwrap();
        assert_eq!(files.len(), 1);

        let links = normalise_links(extract_links(&fs::read_to_string(docs.join("a.md")).unwrap()));
        assert_eq!(links, vec!["missing.md"]);

        let files_set: HashSet<PathBuf> = files.iter().cloned().collect();
        let file_contents: Vec<(PathBuf, String)> = files
            .iter()
            .map(|p| fs::read_to_string(p).map(|c| (p.clone(), c)))
            .collect::<io::Result<_>>()
            .unwrap();
        let link_maps = build_link_maps(
            &vec![NavItem::Page({
                let mut m = HashMap::new();
                m.insert("A".to_string(), "a.md".to_string());
                m
            })],
            root,
        )
        .unwrap();
        let (_refd, broken_direct) =
            analyse_links(&file_contents, &files_set, root, &link_maps).unwrap();
        assert_eq!(broken_direct.len(), 1, "{:?}", broken_direct);

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert_eq!(result.broken_links.len(), 1, "{:?}", result.broken_links);
        assert_eq!(result.broken_links[0].from, docs.join("a.md"));
        assert_eq!(result.broken_links[0].link, "missing.md");
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

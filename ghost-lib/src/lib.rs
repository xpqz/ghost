use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;
use pulldown_cmark::{Event, Parser, Tag};
use scraper::{Html, Selector};

/// Normalize a path by resolving `.` and `..` components without requiring filesystem access.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Pop the last component if it's a Normal component
                if result.last().map_or(false, |c| matches!(c, Component::Normal(_))) {
                    result.pop();
                } else {
                    result.push(component);
                }
            }
            Component::CurDir => {
                // Skip "."
            }
            _ => {
                result.push(component);
            }
        }
    }
    result.into_iter().collect()
}

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
    pub missing_images: Vec<BrokenImage>,
    pub orphan_images: Vec<PathBuf>,
    pub pages_with_footnotes: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BrokenImage {
    pub from: PathBuf,
    pub image: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BrokenLink {
    pub from: PathBuf,
    pub link: String,
    pub from_help_url: bool,
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
    let include_dirs = include_roots(&config.nav, parent);
    markdown_roots.extend(include_dirs.clone());
    let files = find_markdown(markdown_roots)?;
    let files_set: HashSet<PathBuf> = files.iter().cloned().collect();
    let mut ghost = orphans(&pages, &files); // markdown files in the file system not referenced by nav

    let link_maps = build_link_maps(&config.nav, parent)?;

    let help_files = extract_help_urls(help_urls, parent);
    let help_missing = missing_files(&help_files);
    let help_files_set: HashSet<PathBuf> = help_files.iter().cloned().collect();

    // Transitively scan links: start with nav pages AND help_urls references,
    // then follow links to discover more pages
    let mut scanned: HashSet<PathBuf> = HashSet::new();
    let mut to_scan: Vec<PathBuf> = pages
        .iter()
        .chain(help_files.iter())
        .filter(|p| p.is_file())
        .cloned()
        .collect();
    let mut all_referenced: HashSet<PathBuf> = help_files.iter().cloned().collect();
    let mut all_broken_links: Vec<BrokenLink> = Vec::new();

    while !to_scan.is_empty() {
        let file_contents: Vec<(PathBuf, String)> = to_scan
            .iter()
            .filter(|p| !scanned.contains(*p))
            .filter_map(|p| fs::read_to_string(p).ok().map(|c| (p.clone(), c)))
            .collect();

        for (p, _) in &file_contents {
            scanned.insert(p.clone());
        }

        if file_contents.is_empty() {
            break;
        }

        let (referenced, broken_links) =
            analyse_links(&file_contents, &files_set, parent, &include_dirs, &link_maps, &help_files_set)?;

        all_broken_links.extend(broken_links);

        // Find newly discovered files to scan
        to_scan = referenced
            .iter()
            .filter(|p| !scanned.contains(*p) && p.is_file())
            .cloned()
            .collect();

        all_referenced.extend(referenced);
    }

    ghost.retain(|p| !all_referenced.contains(p));

    // Image analysis: find all image assets and check references
    let image_extensions = ["png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp"];
    let all_images: HashSet<PathBuf> = include_dirs
        .iter()
        .flat_map(|dir| {
            WalkDir::new(dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| image_extensions.contains(&ext.to_lowercase().as_str()))
                            .unwrap_or(false)
                })
                .map(|e| normalize_path(e.path()))
        })
        .collect();

    // Find CSS files in include dirs and documentation-assets
    let css_dirs: Vec<PathBuf> = include_dirs
        .iter()
        .cloned()
        .chain(std::iter::once(parent.join("documentation-assets")))
        .filter(|p| p.exists())
        .collect();

    let css_files: Vec<PathBuf> = css_dirs
        .iter()
        .flat_map(|dir| {
            WalkDir::new(dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().is_file()
                        && e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext == "css" || ext == "scss")
                            .unwrap_or(false)
                })
                .map(|e| e.path().to_path_buf())
        })
        .collect();

    // Analyse image references in all scanned markdown files
    let (missing_images, referenced_images) =
        analyse_image_refs(&scanned, &css_files, &all_images, &include_dirs)?;

    // Find orphan images (images not referenced anywhere)
    let orphan_images: Vec<PathBuf> = all_images
        .iter()
        .filter(|img| !referenced_images.contains(*img))
        .cloned()
        .collect();

    // Find pages with footnotes
    let pages_with_footnotes: Vec<PathBuf> = scanned
        .iter()
        .filter(|p| {
            fs::read_to_string(p)
                .map(|content| has_footnotes(&content))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    Ok(AuditResult {
        nav_missing,
        ghost,
        help_missing,
        broken_links: all_broken_links,
        missing_images,
        orphan_images,
        pages_with_footnotes,
    })
}

/// Check if markdown content contains footnote references or definitions.
/// Footnotes use syntax like [^1] for references and [^1]: for definitions.
pub fn has_footnotes(markdown: &str) -> bool {
    // Match footnote references [^identifier] or definitions [^identifier]:
    // The identifier can be alphanumeric with hyphens/underscores
    let footnote_re = Regex::new(r"\[\^[^\]]+\]").unwrap();
    footnote_re.is_match(markdown)
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

/// Extract image references from markdown content.
/// Handles both markdown syntax ![alt](path) and HTML <img src="path">
pub fn extract_image_refs(markdown: &str) -> Vec<String> {
    let mut images = Vec::new();
    let parser = Parser::new(markdown);
    let img_selector = Selector::parse("img[src]").unwrap();

    for event in parser {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                images.push(dest_url.into_string());
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let fragment = Html::parse_fragment(&html);
                for el in fragment.select(&img_selector) {
                    if let Some(src) = el.value().attr("src") {
                        images.push(src.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    images
}

/// Extract image references from CSS content.
/// Handles url() references in background-image, content, etc.
pub fn extract_css_image_refs(css: &str) -> Vec<String> {
    let url_re = Regex::new(r#"url\s*\(\s*['"]?([^'")]+)['"]?\s*\)"#).unwrap();
    url_re
        .captures_iter(css)
        .filter_map(|cap| {
            let url = cap.get(1)?.as_str().trim();
            // Skip data URIs and external URLs
            if url.starts_with("data:") || url.starts_with("http://") || url.starts_with("https://") {
                return None;
            }
            Some(url.to_string())
        })
        .collect()
}

/// Normalise image paths - filter out external URLs
fn normalise_image_refs<I>(refs: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    refs.into_iter()
        .filter(|r| !r.starts_with("http://") && !r.starts_with("https://") && !r.starts_with("data:"))
        .collect()
}

/// Analyse image references in markdown files and CSS files.
/// Returns (missing_images, referenced_images).
fn analyse_image_refs(
    markdown_files: &HashSet<PathBuf>,
    css_files: &[PathBuf],
    all_images: &HashSet<PathBuf>,
    include_dirs: &[PathBuf],
) -> io::Result<(Vec<BrokenImage>, HashSet<PathBuf>)> {
    let mut missing = Vec::new();
    let mut referenced = HashSet::new();

    // Process markdown files
    for src in markdown_files {
        if let Ok(content) = fs::read_to_string(src) {
            let image_refs = normalise_image_refs(extract_image_refs(&content));
            for img_ref in image_refs {
                if let Some(resolved) = resolve_image_ref(src, &img_ref, all_images, include_dirs) {
                    referenced.insert(resolved);
                } else {
                    missing.push(BrokenImage {
                        from: src.clone(),
                        image: img_ref,
                    });
                }
            }
        }
    }

    // Process CSS files
    for css_path in css_files {
        if let Ok(content) = fs::read_to_string(css_path) {
            let image_refs = extract_css_image_refs(&content);
            for img_ref in image_refs {
                if let Some(resolved) = resolve_image_ref(css_path, &img_ref, all_images, include_dirs) {
                    referenced.insert(resolved);
                }
                // Don't report CSS broken images for now - they may reference build artifacts
            }
        }
    }

    Ok((missing, referenced))
}

/// Resolve an image reference to an absolute path.
fn resolve_image_ref(
    src: &Path,
    img_ref: &str,
    all_images: &HashSet<PathBuf>,
    include_dirs: &[PathBuf],
) -> Option<PathBuf> {
    // Handle absolute paths (starting with /)
    if img_ref.starts_with('/') {
        // Try each include dir as potential root
        for dir in include_dirs {
            // The root for absolute paths is typically the docs/ subdirectory
            let docs_dir = dir.join("docs");
            let candidate = if docs_dir.exists() {
                normalize_path(&docs_dir.join(&img_ref[1..]))
            } else {
                normalize_path(&dir.join(&img_ref[1..]))
            };
            if all_images.contains(&candidate) {
                return Some(candidate);
            }
        }
        return None;
    }

    // Handle relative paths
    if let Some(parent) = src.parent() {
        let candidate = normalize_path(&parent.join(img_ref));
        if all_images.contains(&candidate) {
            return Some(candidate);
        }
        // Also check if it exists on disk (might not be in our include_dirs scan)
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Try from each include dir's docs folder
    for dir in include_dirs {
        let docs_dir = dir.join("docs");
        if docs_dir.exists() {
            let candidate = normalize_path(&docs_dir.join(img_ref));
            if all_images.contains(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
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
                for (section, children) in map {
                    let slug = slugify(section);
                    let new_prefix = url_prefix.join(slug);
                    build_link_maps_inner(
                        children,
                        mkdocs_dir,
                        site_root,
                        &new_prefix,
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
    let rendered = if url_prefix.as_os_str().is_empty() {
        normalise_url(&Path::new(nav_path).with_extension(""))
    } else {
        let stem = Path::new(nav_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| nav_path.to_string());
        normalise_url(&url_prefix.join(stem))
    };
    url_to_src.entry(rendered.clone()).or_insert(fs_path.clone());
    src_to_url.entry(fs_path).or_insert(rendered);
}

fn slugify(s: &str) -> String {
    let re = Regex::new(r"[^A-Za-z0-9]+").unwrap();
    let slug = re.replace_all(s, "-");
    slug.trim_matches('-').to_ascii_lowercase()
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
    let target = link.trim_start_matches('/');
    let base_dir = Path::new(from_url)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let mut joined = if link.starts_with('/') {
        PathBuf::from(target)
    } else {
        base_dir.join(target)
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

/// Check if a candidate path exists, falling back to {path_without_ext}/index.md
/// This handles MkDocs convention where `foo.md` can also be `foo/index.md`
fn check_with_index_fallback(candidate: &Path, files_set: &HashSet<PathBuf>) -> Option<PathBuf> {
    // Normalize the candidate path to resolve any ".." components
    let normalized = normalize_path(candidate);

    // Try the path as-is
    if normalized.is_file() || files_set.contains(&normalized) {
        return Some(normalized);
    }

    // Try {stem}/index.md fallback
    let stem = normalized.with_extension("");
    let index_candidate = stem.join("index.md");
    if index_candidate.is_file() || files_set.contains(&index_candidate) {
        return Some(index_candidate);
    }

    None
}

fn analyse_links(
    files: &[(PathBuf, String)],
    files_set: &HashSet<PathBuf>,
    mkdocs_dir: &Path,
    include_dirs: &[PathBuf],
    link_maps: &LinkMaps,
    help_files: &HashSet<PathBuf>,
) -> io::Result<(HashSet<PathBuf>, Vec<BrokenLink>)> {
    let mut referenced = HashSet::new();
    let mut broken_links = Vec::new();

    for (src, content) in files {
        let from_help_url = help_files.contains(src);
        let links = normalise_links(extract_links(content));
        #[cfg(test)]
        eprintln!("analysing {} links for {}", links.len(), src.display());
        for link in links {
            // 1) Try nav-based resolution
            if let Some(target) = resolve_link(src, &link, link_maps) {
                #[cfg(test)]
                eprintln!("resolved via nav: {} -> {}", link, target.display());
                referenced.insert(target);
                continue;
            }

            // 2) Try URL-space resolution (handles cross-subsite links and sibling files)
            // Try both page-as-directory model (how browsers resolve) and parent-dir model
            let url_candidates = resolve_link_via_url_space(src, &link, mkdocs_dir);
            let mut url_resolved = false;
            for candidate in url_candidates {
                if let Some(resolved) = check_with_index_fallback(&candidate, files_set) {
                    #[cfg(test)]
                    eprintln!("resolved via url space: {} -> {}", link, resolved.display());
                    referenced.insert(resolved);
                    url_resolved = true;
                    break;
                }
            }
            if url_resolved {
                continue;
            }

            // 3) Try include directories using rendered URL path
            if let Some(rendered) = rendered_url_for_link(src, &link, link_maps) {
                // same-doc-root guess (if path contains /docs/)
                if let Some(doc_root) = docs_root_for(src) {
                    let candidate = doc_root
                        .join("docs")
                        .join(&rendered)
                        .with_extension("md")
                        .components()
                        .collect::<PathBuf>();
                    if let Some(resolved) = check_with_index_fallback(&candidate, files_set) {
                        #[cfg(test)]
                        eprintln!("resolved via doc root: {} -> {}", link, resolved.display());
                        referenced.insert(resolved);
                        continue;
                    }
                }

                let mut hit = false;
                for dir in include_dirs {
                    let candidate = dir
                        .join("docs")
                        .join(&rendered)
                        .with_extension("md")
                        .components()
                        .collect::<PathBuf>();
                    if let Some(resolved) = check_with_index_fallback(&candidate, files_set) {
                        #[cfg(test)]
                        eprintln!("resolved via include dir: {} -> {}", link, resolved.display());
                        referenced.insert(resolved);
                        hit = true;
                        break;
                    }
                }
                if hit {
                    continue;
                }
            }

            // 4) Final fallback: resolve on filesystem relative to source doc root
            if let Some(fs_candidate) = fs_path_from_link(src, &link) {
                if let Some(resolved) = check_with_index_fallback(&fs_candidate, files_set) {
                    #[cfg(test)]
                    eprintln!("resolved via fs fallback: {} -> {}", link, resolved.display());
                    referenced.insert(resolved);
                    continue;
                }
            }

            // 5) Last resort: plain filesystem relative to source parent
            if let Some(parent) = src.parent() {
                let candidate = parent.join(&link).components().collect::<PathBuf>();
                if let Some(resolved) = check_with_index_fallback(&candidate, files_set) {
                    #[cfg(test)]
                    eprintln!("resolved via parent fallback: {} -> {}", link, resolved.display());
                    referenced.insert(resolved);
                    continue;
                }
            }

            // Unresolved
            #[cfg(test)]
            eprintln!("broken: {} -> {}", src.display(), link);
            broken_links.push(BrokenLink {
                from: src.clone(),
                link: link.clone(),
                from_help_url,
            });
        }
    }

    #[cfg(test)]
    eprintln!("returning broken_links len {}", broken_links.len());
    Ok((referenced, broken_links))
}

fn docs_root_for(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        if let Some(file_name) = ancestor.file_name() {
            if file_name == "docs" {
                return ancestor.parent().map(|p| p.to_path_buf());
            }
        }
    }
    None
}

fn fs_path_from_link(src: &Path, link: &str) -> Option<PathBuf> {
    let link_path = Path::new(link);
    if link_path.is_absolute() {
        let doc_root = docs_root_for(src)?;
        return Some(
            doc_root
                .join("docs")
                .join(link_path.strip_prefix("/").unwrap_or(link_path))
                .components()
                .collect::<PathBuf>(),
        );
    }

    let base = src.parent()?;
    Some(base.join(link_path).components().collect::<PathBuf>())
}

/// Resolve a relative link in URL space, then map back to filesystem.
/// Returns multiple candidates because MkDocs supports two resolution models:
/// 1. Page-as-directory (browser behavior): ../sibling from dir/page/ -> dir/sibling/
/// 2. Filesystem-relative: ../sibling from dir/page.md -> sibling.md (at parent level)
///
/// Example (cross-subsite):
///   src: /base/release-notes/docs/new-enhanced.md
///   link: ../../programming-reference-guide/intro/foo.md
///
///   URL space: release-notes/new-enhanced + ../../programming-reference-guide/intro/foo
///            = programming-reference-guide/intro/foo
///
///   Filesystem: /base/programming-reference-guide/docs/intro/foo.md
///
/// Example (sibling via page-as-directory):
///   src: /base/guide/docs/config/aplan-output.md
///   link: ../aplan-editor.md
///
///   URL space (page-as-dir): guide/config/aplan-output + ../aplan-editor
///                          = guide/config/aplan-editor
///
///   Filesystem: /base/guide/docs/config/aplan-editor.md
fn resolve_link_via_url_space(src: &Path, link: &str, monorepo_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Find the docs/ directory containing src
    let Some(docs_dir) = src.ancestors().find(|a| a.file_name() == Some("docs".as_ref())) else {
        return candidates;
    };
    let Some(subsite_dir) = docs_dir.parent() else {
        return candidates;
    };

    // Compute source's URL: subsite_name + path_within_docs (minus .md)
    let Some(subsite_name) = subsite_dir.file_name().and_then(|s| s.to_str()) else {
        return candidates;
    };
    let Ok(path_within_docs) = src.strip_prefix(docs_dir) else {
        return candidates;
    };
    let src_url_path = Path::new(subsite_name).join(path_within_docs).with_extension("");

    let link_path = Path::new(link).with_extension("");

    // Try both resolution models for relative links
    let base_paths: Vec<&Path> = if link.starts_with('/') {
        vec![] // Absolute links don't need multiple bases
    } else {
        // 1. Page-as-directory model (browser behavior)
        // 2. Parent-directory model (filesystem-like)
        vec![
            src_url_path.as_path(),
            src_url_path.parent().unwrap_or(Path::new("")),
        ]
    };

    // Handle absolute links
    if link.starts_with('/') {
        let resolved = link_path.strip_prefix("/").unwrap_or(&link_path).to_path_buf();
        if let Some(fs_path) = url_to_filesystem(&normalise_url(&resolved), subsite_name, docs_dir, monorepo_root) {
            candidates.push(fs_path);
        }
        return candidates;
    }

    // Try each base path
    for base in base_paths {
        let resolved_url = base.join(&link_path);
        let normalized_url = normalise_url(&resolved_url);
        if normalized_url.is_empty() {
            continue;
        }

        if let Some(fs_path) = url_to_filesystem(&normalized_url, subsite_name, docs_dir, monorepo_root) {
            if !candidates.contains(&fs_path) {
                candidates.push(fs_path);
            }
        }
    }

    candidates
}

/// Map a normalized URL path back to a filesystem path
fn url_to_filesystem(normalized_url: &str, subsite_name: &str, docs_dir: &Path, monorepo_root: &Path) -> Option<PathBuf> {
    let mut url_parts = normalized_url.split('/');
    let first_component = url_parts.next()?;
    let rest: Vec<&str> = url_parts.collect();

    // Check if first_component is a separate subsite (has its own docs/ folder)
    // or just a directory within the current subsite
    let candidate_subsite_docs = monorepo_root.join(first_component).join("docs");

    let fs_path = if first_component != subsite_name && candidate_subsite_docs.is_dir() {
        // Cross-subsite link: insert docs/ after the target subsite
        let mut path = candidate_subsite_docs;
        for part in rest {
            path = path.join(part);
        }
        path
    } else {
        // Same subsite: the resolved URL path is relative to source's docs/
        docs_dir.join(normalized_url.trim_start_matches(&format!("{}/", subsite_name)))
    };

    Some(fs_path.with_extension("md").components().collect())
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
        .filter(|p| {
            // Exclude *-print.md files (print variants of pages)
            !p.file_name()
                .map(|n| n.to_string_lossy().ends_with("-print.md"))
                .unwrap_or(false)
        })
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
        fs::write(docs.join("a.md"), "[Link](orphan.md)").unwrap();
        fs::write(docs.join("orphan.md"), "# Orphan").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - A: a.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(!result.ghost.contains(&docs.join("orphan.md")));
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
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
            analyse_links(&file_contents, &files_set, root, &[], &link_maps, &HashSet::new()).unwrap();
        assert_eq!(broken_direct.len(), 1, "{:?}", broken_direct);

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert_eq!(result.broken_links.len(), 1, "{:?}", result.broken_links);
        assert_eq!(result.broken_links[0].from, docs.join("a.md"));
        assert_eq!(result.broken_links[0].link, "missing.md");
    }


    #[test]
    fn test_relative_parent_link_with_anchor_resolves() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs").join("primitive-operators");
        fs::create_dir_all(&docs).unwrap();
        fs::write(
            docs.join("beside.md"),
            "see [Function Composition](../operator-syntax#function-composition)",
        )
        .unwrap();
        fs::write(root.join("docs").join("operator-syntax.md"), "# Operator Syntax").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - Beside: primitive-operators/beside.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_non_nav_fs_link_resolves() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs").join("primitive-operators");
        fs::create_dir_all(&docs).unwrap();
        fs::write(
            docs.join("beside.md"),
            "see [Function Composition](../operator-syntax#function-composition)",
        )
        .unwrap();
        // target exists on filesystem but is not in nav
        fs::write(root.join("docs").join("operator-syntax.md"), "# Operator Syntax").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - Beside: primitive-operators/beside.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
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

    #[test]
    fn test_adjacent_nav_pages_resolve_parent_link() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let docs = root.join("docs").join("primitive-operators");
        fs::create_dir_all(&docs).unwrap();
        fs::write(
            docs.join("beside.md"),
            "see [Operator Syntax](../operator-syntax.md)",
        )
        .unwrap();
        fs::write(root.join("docs").join("operator-syntax.md"), "# Operator Syntax").unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let mkdocs = r#"
nav:
  - Operator Syntax: operator-syntax.md
  - Beside: primitive-operators/beside.md
"#;
        fs::write(root.join("mkdocs.yml"), mkdocs).unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_cross_subsite_link_resolves() {
        // Simulates monorepo structure:
        //   root/
        //     release-notes/docs/new-enhanced.md  (links to programming-reference-guide)
        //     programming-reference-guide/docs/introduction/arrays/array-notation.md
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        // Create release-notes subsite
        let rn_docs = root.join("release-notes").join("docs");
        fs::create_dir_all(&rn_docs).unwrap();
        fs::write(
            rn_docs.join("new-enhanced.md"),
            "see [Array Notation](../../programming-reference-guide/introduction/arrays/array-notation.md)",
        )
        .unwrap();
        let rn_mkdocs = r#"
nav:
  - New: new-enhanced.md
"#;
        fs::write(root.join("release-notes").join("mkdocs.yml"), rn_mkdocs).unwrap();

        // Create programming-reference-guide subsite
        let prg_docs = root.join("programming-reference-guide").join("docs").join("introduction").join("arrays");
        fs::create_dir_all(&prg_docs).unwrap();
        fs::write(prg_docs.join("array-notation.md"), "# Array Notation").unwrap();
        let prg_mkdocs = r#"
nav:
  - Introduction:
    - Arrays:
      - Array Notation: introduction/arrays/array-notation.md
"#;
        fs::write(root.join("programming-reference-guide").join("mkdocs.yml"), prg_mkdocs).unwrap();

        // Create root mkdocs.yml that includes both subsites
        let root_mkdocs = r#"
nav:
  - Release Notes: '!include ./release-notes/mkdocs.yml'
  - Programming Reference: '!include ./programming-reference-guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_sibling_file_via_parent_link_resolves() {
        // MkDocs treats pages as directories, so ../sibling.md from page `dir/foo`
        // resolves to `dir/sibling`, not to sibling at the parent level
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let docs = root.join("windows-guide").join("docs").join("config-params");
        fs::create_dir_all(&docs).unwrap();

        fs::write(
            docs.join("aplan-for-output.md"),
            "see [Editor](../aplan-for-editor.md)",
        )
        .unwrap();
        fs::write(docs.join("aplan-for-editor.md"), "# Editor").unwrap();

        let mkdocs = r#"
nav:
  - Config:
    - Output: config-params/aplan-for-output.md
    - Editor: config-params/aplan-for-editor.md
"#;
        fs::write(root.join("windows-guide").join("mkdocs.yml"), mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Windows Guide: '!include ./windows-guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_link_to_directory_with_index_resolves() {
        // When linking to `foo.md` but `foo/index.md` exists, it should resolve
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let lrg_docs = root.join("language-reference-guide").join("docs");
        let intro = lrg_docs.join("introduction").join("arrays");
        let ravel_dir = lrg_docs.join("primitive-functions").join("ravel");
        fs::create_dir_all(&intro).unwrap();
        fs::create_dir_all(&ravel_dir).unwrap();

        // Source file links to ravel.md, but ravel is a directory with index.md
        fs::write(
            intro.join("structuring.md"),
            "see [Ravel](../../../../language-reference-guide/primitive-functions/ravel.md)",
        )
        .unwrap();
        fs::write(ravel_dir.join("index.md"), "# Ravel").unwrap();

        let lrg_mkdocs = r#"
nav:
  - Introduction:
    - Arrays:
      - Structuring: introduction/arrays/structuring.md
  - Primitive Functions:
    - Ravel: primitive-functions/ravel/index.md
"#;
        fs::write(root.join("language-reference-guide").join("mkdocs.yml"), lrg_mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Language Reference: '!include ./language-reference-guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_within_subsite_deep_relative_link_resolves() {
        // Simulates link within same subsite that goes up multiple levels:
        //   root/
        //     language-reference-guide/docs/system-functions/shell.md
        //       links to ../../primitive-operators/i-beam/shell-process-control.md
        //     language-reference-guide/docs/primitive-operators/i-beam/shell-process-control.md
        //
        // This should NOT be treated as cross-subsite (primitive-operators is not a subsite)
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        // Create language-reference-guide subsite with nested structure
        let lrg_docs = root.join("language-reference-guide").join("docs");
        let sys_funcs = lrg_docs.join("system-functions");
        let prim_ops = lrg_docs.join("primitive-operators").join("i-beam");
        fs::create_dir_all(&sys_funcs).unwrap();
        fs::create_dir_all(&prim_ops).unwrap();

        fs::write(
            sys_funcs.join("shell.md"),
            "see [Shell Process Control](../../primitive-operators/i-beam/shell-process-control.md)",
        )
        .unwrap();
        fs::write(prim_ops.join("shell-process-control.md"), "# Shell Process Control").unwrap();

        let lrg_mkdocs = r#"
nav:
  - System Functions:
    - Shell: system-functions/shell.md
  - Primitive Operators:
    - I-Beam:
      - Shell Process Control: primitive-operators/i-beam/shell-process-control.md
"#;
        fs::write(root.join("language-reference-guide").join("mkdocs.yml"), lrg_mkdocs).unwrap();

        // Create root mkdocs.yml
        let root_mkdocs = r#"
nav:
  - Language Reference: '!include ./language-reference-guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_absolute_link_resolves() {
        // Absolute links (starting with /) resolve from site root
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let guide_docs = root.join("guide").join("docs");
        let nested = guide_docs.join("nested").join("deep");
        fs::create_dir_all(&nested).unwrap();

        // Deep nested file links to root-level page with absolute path
        fs::write(
            nested.join("page.md"),
            "see [Home](/guide/index.md)",
        )
        .unwrap();
        fs::write(guide_docs.join("index.md"), "# Home").unwrap();

        let mkdocs = r#"
nav:
  - Home: index.md
  - Nested:
    - Deep:
      - Page: nested/deep/page.md
"#;
        fs::write(root.join("guide").join("mkdocs.yml"), mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Guide: '!include ./guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_link_without_extension_resolves() {
        // Links without .md extension should resolve (MkDocs supports this)
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let docs = root.join("guide").join("docs");
        fs::create_dir_all(&docs).unwrap();

        fs::write(
            docs.join("source.md"),
            "see [Target](target)",  // no .md extension
        )
        .unwrap();
        fs::write(docs.join("target.md"), "# Target").unwrap();

        let mkdocs = r#"
nav:
  - Source: source.md
  - Target: target.md
"#;
        fs::write(root.join("guide").join("mkdocs.yml"), mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Guide: '!include ./guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_link_with_trailing_slash_resolves() {
        // Links with trailing slash (directory-style) should resolve
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let docs = root.join("guide").join("docs");
        fs::create_dir_all(&docs).unwrap();

        fs::write(
            docs.join("source.md"),
            "see [Target](target/)",  // trailing slash
        )
        .unwrap();
        fs::write(docs.join("target.md"), "# Target").unwrap();

        let mkdocs = r#"
nav:
  - Source: source.md
  - Target: target.md
"#;
        fs::write(root.join("guide").join("mkdocs.yml"), mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Guide: '!include ./guide/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }

    #[test]
    fn test_normalise_links_filters_correctly() {
        // Unit test for link normalisation logic
        let links = vec![
            "page.md".to_string(),           // already has .md
            "page".to_string(),              // needs .md added
            "dir/page".to_string(),          // needs .md added
            "page#anchor".to_string(),       // anchor should be stripped
            "page.md#anchor".to_string(),    // anchor should be stripped
            "https://example.com".to_string(), // external, should be dropped
            "mailto:test@example.com".to_string(), // mailto, should be dropped
            "path/to/dir/".to_string(),      // trailing slash
            "image.png".to_string(),         // non-md extension, should be dropped
            "#just-anchor".to_string(),      // just anchor, should be dropped
        ];

        let normalised = normalise_links(links);

        assert!(normalised.contains(&"page.md".to_string()));
        assert!(normalised.contains(&"page.md".to_string()));
        assert!(normalised.contains(&"dir/page.md".to_string()));
        assert!(normalised.contains(&"path/to/dir.md".to_string()));
        assert!(!normalised.iter().any(|l| l.contains('#')));
        assert!(!normalised.iter().any(|l| l.starts_with("http")));
        assert!(!normalised.iter().any(|l| l.starts_with("mailto")));
        assert!(!normalised.iter().any(|l| l.ends_with(".png")));
    }

    #[test]
    fn test_extract_links_from_markdown_and_html() {
        // Verify both markdown links and HTML links are extracted
        let content = r#"
# Test Page

Here is a [markdown link](markdown-target.md).

And here is <a href="html-target.md">an HTML link</a>.

And an inline <a href="inline.md">inline link</a> in text.
"#;

        let links = extract_links(content);

        assert!(links.contains(&"markdown-target.md".to_string()));
        assert!(links.contains(&"html-target.md".to_string()));
        assert!(links.contains(&"inline.md".to_string()));
    }

    #[test]
    fn test_extract_links_with_bold_text() {
        // Links with bold text inside should still be extracted
        let content = r#"[**Applies To**](../propertyapplies/accelerator.md)"#;
        let links = extract_links(content);
        eprintln!("Links found: {:?}", links);
        assert!(links.contains(&"../propertyapplies/accelerator.md".to_string()));
    }

    #[test]
    fn test_extract_links_from_markdown_table() {
        // Links inside markdown tables should be extracted
        let content = r#"
|----------------------------------------------|----------------------------------------|
|[ActiveXControl](../objects/activexcontrol.md)|[Bitmap](../objects/bitmap.md)          |
|[ButtonEdit](../objects/buttonedit.md)        |[Calendar](../objects/calendar.md)      |
"#;
        let links = extract_links(content);
        eprintln!("Table links found: {:?}", links);
        assert!(links.contains(&"../objects/activexcontrol.md".to_string()));
        assert!(links.contains(&"../objects/bitmap.md".to_string()));
        assert!(links.contains(&"../objects/buttonedit.md".to_string()));
        assert!(links.contains(&"../objects/calendar.md".to_string()));
    }

    #[test]
    fn test_linked_file_not_orphan() {
        // A file linked from a nav page should not be reported as orphan
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let docs = root.join("object-ref").join("docs");
        let properties = docs.join("properties");
        let propertyapplies = docs.join("propertyapplies");
        fs::create_dir_all(&properties).unwrap();
        fs::create_dir_all(&propertyapplies).unwrap();

        // Nav page links to non-nav page
        fs::write(
            properties.join("accelerator.md"),
            r#"[**Applies To**](../propertyapplies/accelerator.md)"#,
        )
        .unwrap();
        // Non-nav page (should not be orphan because it's linked)
        fs::write(propertyapplies.join("accelerator.md"), "# Applies").unwrap();

        let mkdocs = r#"
nav:
  - Properties:
    - Accelerator: properties/accelerator.md
"#;
        fs::write(root.join("object-ref").join("mkdocs.yml"), mkdocs).unwrap();

        let root_mkdocs = r#"
nav:
  - Object Reference: '!include ./object-ref/mkdocs.yml'
"#;
        fs::write(root.join("mkdocs.yml"), root_mkdocs).unwrap();
        fs::write(root.join("help_urls.h"), "").unwrap();

        let result = audit(&root.join("mkdocs.yml"), &root.join("help_urls.h")).unwrap();

        eprintln!("Ghost files: {:?}", result.ghost);
        eprintln!("Broken links: {:?}", result.broken_links);

        // propertyapplies/accelerator.md should NOT be an orphan
        assert!(
            !result.ghost.iter().any(|p| p.to_string_lossy().contains("propertyapplies")),
            "propertyapplies/accelerator.md should not be an orphan"
        );
        assert!(result.broken_links.is_empty(), "{:?}", result.broken_links);
    }
}

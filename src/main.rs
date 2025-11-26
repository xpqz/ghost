use argh::FromArgs;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(FromArgs, Debug)]
/// Audit MkDocs navigation vs on-disk markdown.
struct Cli {
    #[argh(option, long = "mkdocs-yaml")]
    /// path to the mkdocs.yml file to read
    mkdocs_yaml: PathBuf,

    #[argh(option, long = "help-urls")]
    /// path to the header file containing HELP_URL definitions
    help_urls: PathBuf,
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

fn collect_pages(items: &[NavItem], pages: &mut HashSet<PathBuf>, prefix: &Path) {
    for item in items {
        match item {
            NavItem::Page(map) => {
                for path in map.values() {
                    let full_path = prefix.join("docs").join(path);
                    pages.insert(full_path);
                }
            }
            NavItem::Section(map) => {
                for children in map.values() {
                    collect_pages(children, pages, prefix);
                }
            }
            NavItem::PlainPath(path) => {
                let full_path = prefix.join("docs").join(path);
                pages.insert(full_path);
            }
        }
    }
}

fn extract_help_urls<P1, P2>(path: P1, doc_root: P2) -> Vec<PathBuf>
where
    P1: AsRef<Path>,
    P2: AsRef<Path>,
{
    let content = fs::read_to_string(path).expect("failed to read file");

    // Path prefixes
    let define = Regex::new(r#"#define\s+(\w+)\s+"([^"]+)""#).unwrap();
    let macros: HashMap<&str, &str> = define
        .captures_iter(&content)
        .map(|cap| (cap.get(1).unwrap().as_str(), cap.get(2).unwrap().as_str()))
        .collect();

    // HELP_URL entries
    let url_re = Regex::new(r#"HELP_URL\([^,]+,\s*([^)]+)\)"#).unwrap();

    url_re
        .captures_iter(&content)
        .filter_map(|cap| {
            let raw = cap.get(1).unwrap().as_str().trim();
            let relative_path = expand_url(raw, &macros) + ".md";
            let absolute_path = doc_root.as_ref().join(relative_path);
            Some(absolute_path)
        })
        .collect()
}

fn expand_url(raw: &str, macros: &HashMap<&str, &str>) -> String {
    let mut result = String::new();
    for part in raw.split('"') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(&expanded) = macros.get(trimmed) {
            result.push_str(expanded);
        } else {
            result.push_str(trimmed);
        }
    }
    result
}

fn missing_files<'a, I>(pages: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = &'a PathBuf>,
{
    pages
        .into_iter()
        .filter(|p| !p.is_file())
        .cloned()
        .collect()
}

fn find_markdown<P>(root: P) -> io::Result<Vec<PathBuf>>
where
    P: AsRef<Path>,
{
    WalkDir::new(root)
        .into_iter()
        .map(|res| {
            let entry = res.map_err(walkdir_error)?;
            let path = entry.path();
            if entry.file_type().is_file() && path.extension().is_some_and(|ext| ext == "md") {
                Ok(Some(path.canonicalize()?))
            } else {
                Ok(None)
            }
        })
        .filter_map(|r| r.transpose())
        .collect()
}

fn walkdir_error(err: walkdir::Error) -> io::Error {
    let msg = err.to_string();
    err.into_io_error()
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, msg))
}

fn orphans(nav: &HashSet<PathBuf>, files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|p| !nav.contains(*p))
        .cloned()
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli {
        mkdocs_yaml,
        help_urls,
    } = argh::from_env();

    let contents = fs::read_to_string(&mkdocs_yaml)?;
    let config: MkDocsConfig = serde_yaml::from_str(&contents)?;
    let mut pages = HashSet::<PathBuf>::new();
    let parent = mkdocs_yaml.parent().unwrap();
    collect_pages(&config.nav, &mut pages, parent);
    let missing = missing_files(&pages);
    println!("Missing: {:#?}", missing);

    let files = find_markdown(parent)?;

    let ghost = orphans(&pages, &files);

    let _short_path: Vec<&Path> = ghost
        .iter()
        .filter_map(|s| s.strip_prefix(parent).ok())
        .collect();

    //    println!("Ghost: {:#?}", short_path);

    let doc_root = parent.parent().unwrap();
    let help_files = extract_help_urls(&help_urls, doc_root);

    //    println!("Help URLs: {:#?}", help_files);

    let m = missing_files(&help_files);

    println!("Missing: {:#?}", m);

    Ok(())
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

        collect_pages(&nav, &mut pages, prefix);

        assert_eq!(pages.len(), 1);
        assert!(pages.contains(&PathBuf::from("/tmp/docs/docs/path/to/file.md")));
    }

    #[test]
    fn test_collect_pages_with_plain_path() {
        let nav = vec![NavItem::PlainPath("example.md".to_string())];

        let mut pages = HashSet::new();
        let prefix = Path::new("/tmp/docs");

        collect_pages(&nav, &mut pages, prefix);

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

        collect_pages(&nav, &mut pages, prefix);

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

        collect_pages(&nav, &mut pages, prefix);

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

        collect_pages(&nav, &mut pages, prefix);

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

        collect_pages(&nav, &mut pages, prefix);

        assert_eq!(pages.len(), 0);
    }

    #[test]
    fn test_validate_nav_all_exist() {
        let mut pages = HashSet::new();
        // Use this source file which we know exists
        pages.insert(PathBuf::from("src/main.rs"));

        let missing = validate_nav(&pages);

        assert_eq!(missing.len(), 0);
    }

    #[test]
    fn test_validate_nav_missing_files() {
        let mut pages = HashSet::new();
        pages.insert(PathBuf::from("/nonexistent/file.md"));
        pages.insert(PathBuf::from("/another/missing.md"));

        let missing = validate_nav(&pages);

        assert_eq!(missing.len(), 2);
        assert!(missing.contains(&PathBuf::from("/nonexistent/file.md")));
        assert!(missing.contains(&PathBuf::from("/another/missing.md")));
    }

    #[test]
    fn test_validate_nav_mixed() {
        let mut pages = HashSet::new();
        pages.insert(PathBuf::from("src/main.rs")); // exists
        pages.insert(PathBuf::from("/nonexistent.md")); // doesn't exist

        let missing = validate_nav(&pages);

        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0], PathBuf::from("/nonexistent.md"));
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
        let nav = HashSet::new(); // Empty nav

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

        assert_eq!(orphan_files.len(), 0);
    }

    #[test]
    fn test_extract_help_urls_path_construction() {
        // Simulate: argv[1] = "/Users/stefan/work/dyalog-docs/documentation/language-reference-guide/mkdocs.yml"
        let mkdocs_path = Path::new(
            "/Users/stefan/work/dyalog-docs/documentation/language-reference-guide/mkdocs.yml",
        );
        let parent = mkdocs_path.parent().unwrap(); // language-reference-guide
        let doc_root = parent.parent().unwrap(); // documentation

        // Test that paths are constructed correctly
        let test_path = doc_root.join("object-reference/docs/some-file.md");
        assert_eq!(
            test_path,
            PathBuf::from(
                "/Users/stefan/work/dyalog-docs/documentation/object-reference/docs/some-file.md"
            )
        );
    }
}

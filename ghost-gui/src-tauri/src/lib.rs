use ghost_lib::{audit, AuditResult, BrokenImage, BrokenLink};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
pub struct AuditOptions {
    pub mkdocs_yaml: String,
    pub help_urls: String,
    pub nav_missing: bool,
    pub ghost: bool,
    pub help_missing: bool,
    pub broken_links: bool,
    pub missing_images: bool,
    pub orphan_images: bool,
    pub footnotes: bool,
    pub summary: bool,
    pub exclude: String,
}

#[derive(Debug, Serialize)]
pub struct AuditOutput {
    pub success: bool,
    pub error: Option<String>,
    pub output: String,
    pub counts: AuditCounts,
    pub items: AuditItems,
    pub git_info: Option<GitInfo>,
}

#[derive(Debug, Serialize, Default)]
pub struct AuditItems {
    pub nav_missing: Vec<String>,
    pub ghost: Vec<String>,
    pub help_missing: Vec<String>,
    pub broken_links: Vec<BrokenLinkItem>,
    pub missing_images: Vec<BrokenImageItem>,
    pub orphan_images: Vec<String>,
    pub footnotes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BrokenLinkItem {
    pub from: String,
    pub link: String,
    pub from_help_url: bool,
}

#[derive(Debug, Serialize)]
pub struct BrokenImageItem {
    pub from: String,
    pub image: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct GitInfo {
    pub branch: String,
    pub hash_short: String,
}

fn detect_git_info(dir: &Path) -> Option<GitInfo> {
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    let hash_short = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())?;

    Some(GitInfo { branch, hash_short })
}

#[derive(Debug, Serialize, Default)]
pub struct AuditCounts {
    pub nav_missing: usize,
    pub ghost: usize,
    pub help_missing: usize,
    pub broken_links: usize,
    pub missing_images: usize,
    pub orphan_images: usize,
    pub footnotes: usize,
    pub total: usize,
}

fn relative_path(p: &Path, root: Option<&Path>) -> String {
    if let Some(r) = root {
        p.strip_prefix(r)
            .map(|rel| rel.display().to_string())
            .unwrap_or_else(|_| p.display().to_string())
    } else {
        p.display().to_string()
    }
}

fn is_excluded(p: &Path, root: Option<&Path>, excluded: &[&str]) -> bool {
    if let Some(r) = root {
        if let Ok(rel) = p.strip_prefix(r) {
            if let Some(first) = rel.components().next() {
                let subsite = first.as_os_str().to_string_lossy();
                return excluded.iter().any(|&ex| ex == subsite);
            }
        }
    }
    false
}

fn format_result(
    result: &AuditResult,
    options: &AuditOptions,
    monorepo_root: Option<&Path>,
) -> (String, AuditCounts, AuditItems) {
    let excluded: Vec<&str> = if options.exclude.is_empty() {
        vec![]
    } else {
        options.exclude.split(',').map(|s| s.trim()).collect()
    };

    // Filter results
    let nav_missing: Vec<&PathBuf> = result
        .nav_missing
        .iter()
        .filter(|p| !is_excluded(p, monorepo_root, &excluded))
        .collect();
    let ghost: Vec<&PathBuf> = result
        .ghost
        .iter()
        .filter(|p| !is_excluded(p, monorepo_root, &excluded))
        .collect();
    let help_missing: Vec<&PathBuf> = result
        .help_missing
        .iter()
        .filter(|p| !is_excluded(p, monorepo_root, &excluded))
        .collect();
    let broken_links: Vec<&BrokenLink> = result
        .broken_links
        .iter()
        .filter(|bl| !is_excluded(&bl.from, monorepo_root, &excluded))
        .collect();
    let missing_images: Vec<&BrokenImage> = result
        .missing_images
        .iter()
        .filter(|bi| !is_excluded(&bi.from, monorepo_root, &excluded))
        .collect();
    let orphan_images: Vec<&PathBuf> = result
        .orphan_images
        .iter()
        .filter(|p| !is_excluded(p, monorepo_root, &excluded))
        .collect();
    let footnotes: Vec<&PathBuf> = result
        .pages_with_footnotes
        .iter()
        .filter(|p| !is_excluded(p, monorepo_root, &excluded))
        .collect();

    // Determine which reports to show
    let show_all = !options.nav_missing
        && !options.ghost
        && !options.help_missing
        && !options.broken_links
        && !options.missing_images
        && !options.orphan_images
        && !options.footnotes;

    let show_nav_missing = show_all || options.nav_missing;
    let show_ghost = show_all || options.ghost;
    let show_help_missing = show_all || options.help_missing;
    let show_broken_links = show_all || options.broken_links;
    let show_missing_images = show_all || options.missing_images;
    let show_orphan_images = show_all || options.orphan_images;
    let show_footnotes = options.footnotes;

    let mut output = String::new();
    let mut counts = AuditCounts::default();

    if show_nav_missing {
        counts.nav_missing = nav_missing.len();
        format_pathbuf_section(
            &mut output,
            "Missing nav entries",
            &nav_missing,
            options.summary,
            monorepo_root,
        );
    }

    if show_ghost {
        counts.ghost = ghost.len();
        format_pathbuf_section(
            &mut output,
            "Ghost files (orphans)",
            &ghost,
            options.summary,
            monorepo_root,
        );
    }

    if show_help_missing {
        counts.help_missing = help_missing.len();
        format_pathbuf_section(
            &mut output,
            "Missing help URLs",
            &help_missing,
            options.summary,
            monorepo_root,
        );
    }

    if show_broken_links {
        counts.broken_links = broken_links.len();
        format_broken_links_section(
            &mut output,
            "Broken links",
            &broken_links,
            options.summary,
            monorepo_root,
        );
    }

    if show_missing_images {
        counts.missing_images = missing_images.len();
        format_broken_images_section(
            &mut output,
            "Missing images",
            &missing_images,
            options.summary,
            monorepo_root,
        );
    }

    if show_orphan_images {
        counts.orphan_images = orphan_images.len();
        format_pathbuf_section(
            &mut output,
            "Orphan images",
            &orphan_images,
            options.summary,
            monorepo_root,
        );
    }

    if show_footnotes {
        counts.footnotes = footnotes.len();
        format_pathbuf_section(
            &mut output,
            "Pages with footnotes",
            &footnotes,
            options.summary,
            monorepo_root,
        );
    }

    counts.total = counts.nav_missing
        + counts.ghost
        + counts.help_missing
        + counts.broken_links
        + counts.missing_images
        + counts.orphan_images;

    if !options.summary {
        output.push_str(&format!("\nTotal issues: {}\n", counts.total));
    }

    // Build items for rich view
    let mut items = AuditItems::default();

    if show_nav_missing {
        items.nav_missing = nav_missing
            .iter()
            .map(|p| relative_path(p, monorepo_root))
            .collect();
    }

    if show_ghost {
        items.ghost = ghost
            .iter()
            .map(|p| relative_path(p, monorepo_root))
            .collect();
    }

    if show_help_missing {
        items.help_missing = help_missing
            .iter()
            .map(|p| relative_path(p, monorepo_root))
            .collect();
    }

    if show_broken_links {
        items.broken_links = broken_links
            .iter()
            .map(|bl| BrokenLinkItem {
                from: relative_path(&bl.from, monorepo_root),
                link: bl.link.clone(),
                from_help_url: bl.from_help_url,
            })
            .collect();
    }

    if show_missing_images {
        items.missing_images = missing_images
            .iter()
            .map(|bi| BrokenImageItem {
                from: relative_path(&bi.from, monorepo_root),
                image: bi.image.clone(),
            })
            .collect();
    }

    if show_orphan_images {
        items.orphan_images = orphan_images
            .iter()
            .map(|p| relative_path(p, monorepo_root))
            .collect();
    }

    if show_footnotes {
        items.footnotes = footnotes
            .iter()
            .map(|p| relative_path(p, monorepo_root))
            .collect();
    }

    (output, counts, items)
}

fn format_pathbuf_section(
    output: &mut String,
    title: &str,
    items: &[&PathBuf],
    summary: bool,
    monorepo_root: Option<&Path>,
) {
    if summary {
        output.push_str(&format!("{}: {}\n", title, items.len()));
    } else {
        output.push_str(&format!("\n{}:\n", title));
        if items.is_empty() {
            output.push_str("  (none)\n");
        } else {
            for item in items {
                output.push_str(&format!("  {}\n", relative_path(item, monorepo_root)));
            }
        }
    }
}

fn format_broken_links_section(
    output: &mut String,
    title: &str,
    items: &[&BrokenLink],
    summary: bool,
    monorepo_root: Option<&Path>,
) {
    if summary {
        output.push_str(&format!("{}: {}\n", title, items.len()));
    } else {
        output.push_str(&format!("\n{}:\n", title));
        if items.is_empty() {
            output.push_str("  (none)\n");
        } else {
            for bl in items {
                let marker = if bl.from_help_url { "[H] " } else { "" };
                output.push_str(&format!(
                    "  {}{} -> {}\n",
                    marker,
                    relative_path(&bl.from, monorepo_root),
                    bl.link
                ));
            }
        }
    }
}

fn format_broken_images_section(
    output: &mut String,
    title: &str,
    items: &[&BrokenImage],
    summary: bool,
    monorepo_root: Option<&Path>,
) {
    if summary {
        output.push_str(&format!("{}: {}\n", title, items.len()));
    } else {
        output.push_str(&format!("\n{}:\n", title));
        if items.is_empty() {
            output.push_str("  (none)\n");
        } else {
            for bi in items {
                output.push_str(&format!(
                    "  {} -> {}\n",
                    relative_path(&bi.from, monorepo_root),
                    bi.image
                ));
            }
        }
    }
}

#[tauri::command]
fn get_home_dir() -> Option<String> {
    // Only return home dir on Unix systems where ~ is a recognized convention
    #[cfg(unix)]
    {
        dirs::home_dir().map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[tauri::command]
fn open_in_editor(file_path: String) -> Result<(), String> {
    // Try to open with 'code' command (VS Code CLI)
    let result = Command::new("code")
        .arg(&file_path)
        .spawn();

    match result {
        Ok(_) => Ok(()),
        Err(_) => {
            // Fallback: try macOS 'open' command
            #[cfg(target_os = "macos")]
            {
                Command::new("open")
                    .arg(&file_path)
                    .spawn()
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err(e.to_string())
            }
        }
    }
}

#[tauri::command]
fn run_audit(options: AuditOptions) -> AuditOutput {
    let mkdocs_path = PathBuf::from(&options.mkdocs_yaml);
    let help_urls_path = PathBuf::from(&options.help_urls);

    let monorepo_root = mkdocs_path.parent().map(|p| p.to_path_buf());
    let git_info = monorepo_root.as_deref().and_then(detect_git_info);

    match audit(&mkdocs_path, &help_urls_path) {
        Ok(result) => {
            let (output, counts, items) = format_result(&result, &options, monorepo_root.as_deref());
            AuditOutput {
                success: true,
                error: None,
                output,
                counts,
                items,
                git_info,
            }
        }
        Err(e) => AuditOutput {
            success: false,
            error: Some(e.to_string()),
            output: String::new(),
            counts: AuditCounts::default(),
            items: AuditItems::default(),
            git_info,
        },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![run_audit, get_home_dir, open_in_editor])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

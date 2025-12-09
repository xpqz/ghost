use argh::FromArgs;
use ghost_lib::audit;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(FromArgs, Debug)]
/// Audit MkDocs navigation vs on-disk markdown.
///
/// Reports on missing nav entries, orphaned files, broken links, and missing help URLs.
/// By default, shows all report types. Use flags to show only specific reports.
struct Cli {
    #[argh(option, long = "mkdocs-yaml")]
    /// path to the mkdocs.yml file to read
    mkdocs_yaml: PathBuf,

    #[argh(option, long = "help-urls")]
    /// path to the header file containing HELP_URL definitions
    help_urls: PathBuf,

    #[argh(switch, long = "nav-missing")]
    /// show files referenced in nav that don't exist on disk
    nav_missing: bool,

    #[argh(switch, long = "ghost")]
    /// show markdown files on disk not referenced by nav (orphans)
    ghost: bool,

    #[argh(switch, long = "help-missing")]
    /// show files referenced in help_urls.h that don't exist
    help_missing: bool,

    #[argh(switch, long = "broken-links")]
    /// show broken internal links in markdown files
    broken_links: bool,

    #[argh(switch, long = "missing-images")]
    /// show image references that point to non-existent files
    missing_images: bool,

    #[argh(switch, long = "orphan-images")]
    /// show image files not referenced by any markdown or CSS
    orphan_images: bool,

    #[argh(switch, long = "summary")]
    /// show only summary counts, not individual items
    summary: bool,

    #[argh(switch, long = "quiet", short = 'q')]
    /// suppress output, exit with non-zero if any issues found
    quiet: bool,

    #[argh(option, long = "exclude")]
    /// comma-separated list of subsites to exclude from all checks
    exclude: Option<String>,
}

fn main() -> ExitCode {
    let cli: Cli = argh::from_env();

    // Get the monorepo root (parent of mkdocs.yml) for relative path display
    let monorepo_root = cli.mkdocs_yaml.parent().map(|p| p.to_path_buf());

    let result = match audit(&cli.mkdocs_yaml, &cli.help_urls) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Parse excluded subsites
    let excluded: Vec<&str> = cli
        .exclude
        .as_deref()
        .map(|s| s.split(',').map(|x| x.trim()).collect())
        .unwrap_or_default();

    // Helper to check if a path is in an excluded subsite
    let is_excluded = |p: &PathBuf| -> bool {
        if let Some(ref root) = monorepo_root {
            if let Ok(rel) = p.strip_prefix(root) {
                if let Some(first_component) = rel.components().next() {
                    let subsite = first_component.as_os_str().to_string_lossy();
                    return excluded.iter().any(|&ex| ex == subsite);
                }
            }
        }
        false
    };

    // Helper to display paths relative to monorepo root
    let relative_path = |p: &PathBuf| -> String {
        if let Some(ref root) = monorepo_root {
            p.strip_prefix(root)
                .map(|rel| rel.display().to_string())
                .unwrap_or_else(|_| p.display().to_string())
        } else {
            p.display().to_string()
        }
    };

    // If no specific flags are set, show all reports
    let show_all = !cli.nav_missing
        && !cli.ghost
        && !cli.help_missing
        && !cli.broken_links
        && !cli.missing_images
        && !cli.orphan_images;

    let show_nav_missing = show_all || cli.nav_missing;
    let show_ghost = show_all || cli.ghost;
    let show_help_missing = show_all || cli.help_missing;
    let show_broken_links = show_all || cli.broken_links;
    let show_missing_images = show_all || cli.missing_images;
    let show_orphan_images = show_all || cli.orphan_images;

    let mut total_issues = 0;

    // Filter results to exclude specified subsites
    let nav_missing: Vec<_> = result.nav_missing.iter().filter(|p| !is_excluded(p)).collect();
    let ghost: Vec<_> = result.ghost.iter().filter(|p| !is_excluded(p)).collect();
    let help_missing: Vec<_> = result.help_missing.iter().filter(|p| !is_excluded(p)).collect();
    let broken_links: Vec<_> = result.broken_links.iter().filter(|bl| !is_excluded(&bl.from)).collect();
    let missing_images: Vec<_> = result.missing_images.iter().filter(|bi| !is_excluded(&bi.from)).collect();
    let orphan_images: Vec<_> = result.orphan_images.iter().filter(|p| !is_excluded(p)).collect();

    if show_nav_missing {
        total_issues += nav_missing.len();
        if !cli.quiet {
            print_section(
                "Missing nav entries",
                &nav_missing,
                cli.summary,
                |p| relative_path(p),
            );
        }
    }

    if show_ghost {
        total_issues += ghost.len();
        if !cli.quiet {
            print_section(
                "Ghost files (orphans)",
                &ghost,
                cli.summary,
                |p| relative_path(p),
            );
        }
    }

    if show_help_missing {
        total_issues += help_missing.len();
        if !cli.quiet {
            print_section(
                "Missing help URLs",
                &help_missing,
                cli.summary,
                |p| relative_path(p),
            );
        }
    }

    if show_broken_links {
        total_issues += broken_links.len();
        if !cli.quiet {
            print_section(
                "Broken links",
                &broken_links,
                cli.summary,
                |bl| {
                    let marker = if bl.from_help_url { "[H] " } else { "" };
                    format!("{}{} -> {}", marker, relative_path(&bl.from), bl.link)
                },
            );
        }
    }

    if show_missing_images {
        total_issues += missing_images.len();
        if !cli.quiet {
            print_section(
                "Missing images",
                &missing_images,
                cli.summary,
                |bi| format!("{} -> {}", relative_path(&bi.from), bi.image),
            );
        }
    }

    if show_orphan_images {
        total_issues += orphan_images.len();
        if !cli.quiet {
            print_section(
                "Orphan images",
                &orphan_images,
                cli.summary,
                |p| relative_path(p),
            );
        }
    }

    if !cli.quiet && !cli.summary {
        println!();
        println!("Total issues: {}", total_issues);
    }

    if total_issues > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_section<T, F>(title: &str, items: &[T], summary_only: bool, format: F)
where
    F: Fn(&T) -> String,
{
    if summary_only {
        println!("{}: {}", title, items.len());
    } else {
        println!("\n{}:", title);
        if items.is_empty() {
            println!("  (none)");
        } else {
            for item in items {
                println!("  {}", format(item));
            }
        }
    }
}

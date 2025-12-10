# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`ghost` is a Rust tool that validates MkDocs navigation configurations against the actual filesystem. It parses `mkdocs.yml` files to extract navigation structures, then checks for:
- Missing pages (files referenced in nav that don't exist)
- Orphaned files (markdown files in docs/ that aren't included in nav)

## Commands

### Using Just (Recommended)
The project includes a `justfile` for common workflows:

```bash
just check          # Run all checks (format, clippy, tests)
just dev            # Format, lint, and test (quick dev loop)
just test           # Run all tests
just test-one NAME  # Run specific test
just watch          # Auto-run tests on file changes
just clippy-fix     # Auto-fix linting issues
just run PATH       # Run ghost with a mkdocs.yml file
just run-obj        # Test with object-reference
just run-lang       # Test with language-reference-guide
```

### Direct Cargo Commands
```bash
cargo run <path-to-mkdocs.yml>  # Run with file
cargo build                      # Debug build
cargo build --release            # Release build
cargo clippy                     # Run linter
cargo clippy --fix               # Auto-fix linter issues
cargo test                       # Run all tests
cargo test <name>                # Run specific test by name
```

## Architecture

### Data Model

The nav structure in MkDocs YAML is recursively parsed into three variants:
- `NavItem::Page(HashMap<String, String>)` - Maps title to file path (e.g., `"Title": "path.md"`)
- `NavItem::Section(HashMap<String, Vec<NavItem>>)` - Maps section name to child items
- `NavItem::PlainPath(String)` - Path without explicit title (e.g., `"path.md"` - MkDocs derives title from filename)

### Core Flow

1. **Parse** (`main.rs:70-78`): Deserialize YAML into `MkDocsConfig` with serde
2. **Collect** (`collect_pages`): Recursively traverse nav tree, building a `HashSet<String>` of all referenced paths with fully-qualified paths (prefix + "docs/" + path)
3. **Validate** (`validate_nav`): Check each collected path exists on filesystem
4. **Scan** (`find_markdown`): Use `walkdir` to find all `.md` files in docs directory
5. **Detect Orphans** (`orphans`): Find markdown files not present in nav set

### Key Dependencies

- `serde` + `serde_yaml`: YAML deserialization with untagged enum for nav items
- `walkdir`: Recursive directory traversal for markdown file discovery
- Standard library: `HashMap`, `HashSet`, `Path`, `PathBuf` for file operations

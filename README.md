# ghost

A tool for auditing MkDocs documentation sites, particularly those using the monorepo plugin. It validates navigation configurations against the filesystem and checks for broken internal links.

## Features

- **Missing nav entries**: Files referenced in `mkdocs.yml` nav that don't exist on disk
- **Ghost files (orphans)**: Markdown files on disk not referenced by nav
- **Missing help URLs**: Files referenced in a C header file (`HELP_URL` macros) that don't exist
- **Broken links**: Internal markdown links that don't resolve to valid targets
- **Missing images**: Image references in markdown that point to non-existent files
- **Orphan images**: Image files on disk not referenced by any markdown or CSS

## Installation

### CLI

```bash
cargo build --release -p ghost-cli
```

The binary will be at `target/release/ghost`.

### GUI (Tauri)

The GUI provides a cross-platform desktop application with file pickers and interactive results.

**Features:**
- File browser for selecting `mkdocs.yml` and `help_urls.h`
- Checkbox selection for report types
- Rich view with clickable links to open documentation pages in browser
- Raw view for copying output to GitHub issues
- Remembers file selections between sessions

#### Pre-built binaries

Download the latest release for your platform:

| Platform | File | Notes |
|----------|------|-------|
| macOS (Apple Silicon) | `Ghost_x.x.x_aarch64.dmg` | Mount DMG, drag to Applications |
| macOS (Intel) | `Ghost_x.x.x_x64.dmg` | Mount DMG, drag to Applications |
| Windows | `Ghost_x.x.x_x64-setup.exe` | Run installer |

**Note (macOS unsigned apps):** The app is not signed/notarized, so macOS Gatekeeper may report it as "damaged". To fix this, run:
```bash
xattr -cr /Applications/Ghost.app
```

#### Building from source

Prerequisites:
- Rust toolchain
- Platform-specific dependencies for Tauri

**Linux (Debian/Ubuntu):**
```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

**Linux (Fedora):**
```bash
sudo dnf install webkit2gtk4.1-devel openssl-devel curl wget file \
  libappindicator-gtk3-devel librsvg2-devel
sudo dnf group install "C Development Tools and Libraries"
```

**Linux (Arch):**
```bash
sudo pacman -S webkit2gtk-4.1 base-devel curl wget file openssl \
  libappindicator-gtk3 librsvg
```

**macOS:** No additional dependencies (Xcode command line tools required).

**Windows:** No additional dependencies (Visual Studio Build Tools required).

See [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for full details.

Build the release app:

```bash
just gui-build
# Or directly:
cd ghost-gui && cargo tauri build
```

Output locations:
- **macOS**: `target/release/bundle/macos/Ghost.app` and `target/release/bundle/dmg/Ghost_x.x.x_aarch64.dmg`
- **Linux**: `target/release/bundle/deb/` (.deb) and `target/release/bundle/appimage/` (.AppImage)
- **Windows**: `target/release/bundle/msi/` and `target/release/bundle/nsis/`

For development with hot reload:

```bash
just gui-dev
# Or directly:
cd ghost-gui && cargo tauri dev
```

## Usage

```
ghost --mkdocs-yaml <path> --help-urls <path> [options]
```

### Required arguments

| Argument | Description |
|----------|-------------|
| `--mkdocs-yaml <path>` | Path to the root `mkdocs.yml` file |
| `--help-urls <path>` | Path to C header file containing `HELP_URL` definitions |

### Report selection

By default, all report types are shown. Use these flags to show only specific reports:

| Flag | Description |
|------|-------------|
| `--nav-missing` | Show files referenced in nav that don't exist on disk |
| `--ghost` | Show markdown files on disk not referenced by nav |
| `--help-missing` | Show files referenced in help_urls.h that don't exist |
| `--broken-links` | Show broken internal links in markdown files |
| `--missing-images` | Show image references that point to non-existent files |
| `--orphan-images` | Show image files not referenced by any markdown or CSS |

Flags can be combined to show multiple report types.

### Output control

| Flag | Description |
|------|-------------|
| `--summary` | Show only counts, not individual items |
| `-q, --quiet` | Suppress all output, exit with non-zero if issues found |

### Exit codes

| Code | Meaning |
|------|---------|
| `0` | No issues found (for selected report types) |
| `1` | Issues found or error occurred |

## Examples

Show all reports with full details:

```bash
ghost --mkdocs-yaml docs/mkdocs.yml --help-urls src/help_urls.h
```

Show only broken links:

```bash
ghost --mkdocs-yaml docs/mkdocs.yml --help-urls src/help_urls.h --broken-links
```

Show summary counts for all report types:

```bash
ghost --mkdocs-yaml docs/mkdocs.yml --help-urls src/help_urls.h --summary
```

CI check for broken links (silent, uses exit code):

```bash
ghost --mkdocs-yaml docs/mkdocs.yml --help-urls src/help_urls.h --broken-links --quiet
```

Check multiple report types:

```bash
ghost --mkdocs-yaml docs/mkdocs.yml --help-urls src/help_urls.h --broken-links --nav-missing
```

## Monorepo support

Ghost understands MkDocs monorepo structures where multiple subsites are combined via `!include` directives:

```yaml
# Root mkdocs.yml
nav:
  - Guide: '!include ./guide/mkdocs.yml'
  - Reference: '!include ./reference/mkdocs.yml'
```

Each subsite has its own `docs/` directory, and ghost correctly resolves cross-subsite links.

## Link resolution

Ghost validates links as they would work in the rendered MkDocs site. This includes:

- Relative links (`../sibling.md`, `./child.md`)
- Absolute links (`/guide/page.md`)
- Links without extensions (`page` resolves to `page.md`)
- Directory-style links (`dir/` resolves to `dir.md` or `dir/index.md`)
- Cross-subsite links in monorepo setups
- Anchor stripping (`page.md#section` validates `page.md`)

See [LINKS.md](LINKS.md) for detailed documentation of the link resolution algorithm.

## Help URL format

The `--help-urls` file should be a C header with `HELP_URL` macro definitions:

```c
#define LRG "language-reference-guide"
#define SY LRG"/symbols"

HELP_URL(",", SY"/comma")
HELP_URL(":if", "programming-reference-guide/control-structures/if")
```

Ghost expands macros and validates that referenced documentation pages exist.

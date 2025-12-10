# List all available recipes
default:
    @just --list

# Build the project in debug mode
build:
    cargo build

# Build optimized release binary
release:
    cargo build --release

# Run all tests
test:
    cargo test

# Run tests with output visible
test-verbose:
    cargo test -- --nocapture

# Run a specific test by name
test-one NAME:
    cargo test {{NAME}} -- --nocapture

# Run clippy linter
clippy:
    cargo clippy -- -D warnings

# Auto-fix clippy warnings where possible
clippy-fix:
    cargo clippy --fix --allow-dirty

# Format all code
fmt:
    cargo fmt

# Check if code is formatted correctly
fmt-check:
    cargo fmt -- --check

# Run all checks (formatting, clippy, tests) - use before committing
check: fmt-check clippy test
    @echo "✓ All checks passed!"

# Auto-format and run checks
dev: fmt clippy test
    @echo "✓ Development checks complete!"

# Watch for changes and run tests automatically
watch:
    cargo watch -x test

# Watch for changes and run clippy automatically
watch-clippy:
    cargo watch -x clippy

# Clean all build artifacts
clean:
    cargo clean

# Run ghost with the object-reference mkdocs.yml
run-obj:
    cargo run -- --mkdocs-yaml /Users/stefan/work/dyalog-docs/documentation/object-reference/mkdocs.yml --help-urls help_urls.h

# Run ghost with the language-reference-guide mkdocs.yml
run-lang:
    cargo run -- --mkdocs-yaml /Users/stefan/work/dyalog-docs/documentation/language-reference-guide/mkdocs.yml --help-urls help_urls.h

# Run ghost with custom path
run MKDOCS HELP:
    cargo run -- --mkdocs-yaml {{MKDOCS}} --help-urls {{HELP}}

# Build and install ghost to ~/.cargo/bin
install:
    cargo install --path .

# === GUI Commands ===

# Run the GUI in development mode (hot reload)
gui-dev:
    cd ghost-gui && cargo tauri dev

# Build the GUI for release (creates .app and .dmg on macOS)
gui-build:
    cd ghost-gui && cargo tauri build

# Open the built DMG location in Finder (macOS)
gui-open-bundle:
    open target/release/bundle/dmg/

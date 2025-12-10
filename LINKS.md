# Link Resolution Algorithm

This document describes how `ghost` validates markdown links in MkDocs monorepo documentation sites.

## Overview

MkDocs with the monorepo plugin creates a **virtual URL hierarchy** that differs from the **filesystem hierarchy**. The key difference is that each subsite has a `docs/` directory that is stripped from rendered URLs:

```
Filesystem:  subsite/docs/path/to/page.md
Rendered URL: /subsite/path/to/page/
```

Links in markdown source files can be written in several ways, and `ghost` must validate them against what would actually work in the rendered site.

## Link Normalisation

Before resolution, links are normalised:

1. **Anchor stripping**: `page.md#section` → `page.md`
2. **External links skipped**: URLs starting with `http` or `mailto:` are ignored
3. **Trailing slash handling**: `path/to/dir/` → `path/to/dir.md`
4. **Extension normalisation**: Links without extensions get `.md` appended

**Test**: `test_relative_parent_link_with_anchor_resolves` - verifies anchor stripping

## Resolution Strategy

Links are resolved using a multi-phase approach. Each phase is tried in order until one succeeds:

### Phase 1: Nav-based Resolution

If both the source file and target are in the nav structure, resolve using the nav's URL mapping.

The nav defines a virtual URL hierarchy where section names become URL path components. Links are resolved relative to the source page's position in this hierarchy.

**Test**: `test_build_link_maps_with_include_and_resolution` - verifies nav-based URL mapping

### Phase 2: URL-space Resolution

For links not resolvable via nav (e.g., to orphan pages), resolve in URL space using two models:

#### Model A: Page-as-Directory (Browser Behaviour)

MkDocs renders `dir/page.md` as URL `dir/page/`. Browsers resolve relative links from this directory-like URL:

```
Source:  config/aplan-output.md  →  URL: config/aplan-output/
Link:    ../aplan-editor.md
Result:  config/aplan-editor/    →  File: config/aplan-editor.md
```

The `../` goes up from `aplan-output/` to `config/`, then `aplan-editor` is appended.

**Test**: `test_sibling_file_via_parent_link_resolves` - verifies sibling file linking

#### Model B: Parent-Directory (Filesystem-like)

Some links are written with filesystem semantics, where `../` means "parent directory of this file":

```
Source:  programming-reference-guide/docs/native-files.md
Link:    ../language-reference-guide/system-functions/nget.md
Result:  language-reference-guide/docs/system-functions/nget.md
```

**Test**: `test_cross_subsite_link_resolves` - verifies cross-subsite navigation

Both models are tried; the first to resolve to an existing file wins.

### Phase 3: Include Directory Resolution

For pages in nav, compute the rendered URL path and check against each subsite's `docs/` directory.

### Phase 4: Filesystem Fallback

Resolve the link relative to the source file's doc root (the `docs/` directory containing it).

### Phase 5: Parent Directory Fallback

As a last resort, resolve relative to the source file's immediate parent directory.

**Test**: `test_non_nav_fs_link_resolves` - verifies filesystem fallback for non-nav pages

## Special Cases

### Cross-subsite Links

When a link traverses from one subsite to another:

```
Source:  release-notes/docs/new-enhanced.md
Link:    ../../programming-reference-guide/introduction/arrays/array-notation.md
Target:  programming-reference-guide/docs/introduction/arrays/array-notation.md
```

The algorithm:
1. Compute source URL: `release-notes/new-enhanced`
2. Apply relative link in URL space
3. Detect target is a different subsite (has its own `docs/` folder)
4. Insert `docs/` after the subsite name when mapping back to filesystem

**Test**: `test_cross_subsite_link_resolves`

### Within-subsite Deep Links

When a link uses multiple `../` but stays within the same subsite:

```
Source:  language-reference-guide/docs/system-functions/shell.md
Link:    ../../primitive-operators/i-beam/shell-process-control.md
Target:  language-reference-guide/docs/primitive-operators/i-beam/shell-process-control.md
```

The algorithm detects that `primitive-operators` is NOT a separate subsite (no `primitive-operators/docs/` exists at the monorepo root), so it resolves within the current subsite's `docs/` directory.

**Test**: `test_within_subsite_deep_relative_link_resolves`

### Directory Index Files

MkDocs allows `foo.md` to resolve to `foo/index.md`:

```
Link:    primitive-functions/ravel.md
Target:  primitive-functions/ravel/index.md  (when ravel.md doesn't exist)
```

**Test**: `test_link_to_directory_with_index_resolves`

### Sibling Files via Parent Link

Due to page-as-directory URL rendering, `../sibling.md` from `dir/page.md` resolves to `dir/sibling.md`, not to the parent directory:

```
Source:  config-params/aplan-for-output.md  →  URL: config-params/aplan-for-output/
Link:    ../aplan-for-editor.md
Result:  config-params/aplan-for-editor.md
```

**Test**: `test_sibling_file_via_parent_link_resolves`

## Algorithm Summary

```
validate_link(source_file, link):
    link = normalise(link)  // strip anchor, add .md if needed

    if external(link):
        return VALID

    // Phase 1: Nav-based
    if source in nav_map:
        target_url = resolve_relative(source_url, link)
        if target_url in nav_map:
            return VALID

    // Phase 2: URL-space (try both models)
    for base in [source_url_as_dir, source_url_parent]:
        resolved = normalise(base + link)
        fs_path = url_to_filesystem(resolved)
        if exists(fs_path) or exists(fs_path_as_index):
            return VALID

    // Phase 3-5: Fallbacks
    for fallback in [doc_root, include_dirs, fs_relative, parent_relative]:
        if fallback.resolve(link).exists():
            return VALID

    return BROKEN
```

## Subsite Detection

A directory is considered a subsite if `{monorepo_root}/{dir}/docs/` exists. This determines whether cross-directory links should insert `docs/` in the target path.

## Test Coverage

| Scenario | Test |
|----------|------|
| Nav-based resolution | `test_build_link_maps_with_include_and_resolution` |
| Anchor stripping | `test_relative_parent_link_with_anchor_resolves` |
| Cross-subsite links | `test_cross_subsite_link_resolves` |
| Within-subsite deep links | `test_within_subsite_deep_relative_link_resolves` |
| Sibling files via `../` | `test_sibling_file_via_parent_link_resolves` |
| Directory index fallback | `test_link_to_directory_with_index_resolves` |
| Filesystem fallback | `test_non_nav_fs_link_resolves` |
| Adjacent nav pages | `test_adjacent_nav_pages_resolve_parent_link` |
| Broken link detection | `test_broken_link_reported` |
| Ghost removal when linked | `test_ghost_removed_when_linked` |
| Absolute links (`/path`) | `test_absolute_link_resolves` |
| Links without extension | `test_link_without_extension_resolves` |
| Trailing slash links | `test_link_with_trailing_slash_resolves` |
| Link normalisation | `test_normalise_links_filters_correctly` |
| Markdown & HTML extraction | `test_extract_links_from_markdown_and_html` |

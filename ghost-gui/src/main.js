const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;
const { open: shellOpen } = window.__TAURI__.shell;

// Base URL for documentation site (version is appended dynamically)
const DOCS_BASE = 'https://dyalog.github.io/documentation';

// Storage keys
const STORAGE_MKDOCS = 'ghost_mkdocs_path';
const STORAGE_HELP_URLS = 'ghost_help_urls_path';

// Elements
const mkdocsPathInput = document.getElementById('mkdocs-path');
const helpUrlsPathInput = document.getElementById('help-urls-path');
const browseMkdocsBtn = document.getElementById('browse-mkdocs');
const browseHelpUrlsBtn = document.getElementById('browse-help-urls');
const runAuditBtn = document.getElementById('run-audit');
const resultsSection = document.getElementById('results-section');
const countsDiv = document.getElementById('counts');
const outputPre = document.getElementById('output');
const richOutputDiv = document.getElementById('rich-output');
const viewRichRadio = document.getElementById('view-rich');
const viewRawRadio = document.getElementById('view-raw');
const versionInput = document.getElementById('version');
const gitInfoEl = document.getElementById('git-info');

// Checkboxes
const optNavMissing = document.getElementById('opt-nav-missing');
const optGhost = document.getElementById('opt-ghost');
const optHelpMissing = document.getElementById('opt-help-missing');
const optBrokenLinks = document.getElementById('opt-broken-links');
const optMissingImages = document.getElementById('opt-missing-images');
const optOrphanImages = document.getElementById('opt-orphan-images');
const optFootnotes = document.getElementById('opt-footnotes');
const optSummary = document.getElementById('opt-summary');
const excludeInput = document.getElementById('exclude');

// Report type checkboxes (not including summary)
const reportCheckboxes = [optNavMissing, optGhost, optHelpMissing, optBrokenLinks, optMissingImages, optOrphanImages, optFootnotes];

// Checkbox logic: summary and report types are mutually exclusive
optSummary.addEventListener('change', () => {
  if (optSummary.checked) {
    reportCheckboxes.forEach(cb => cb.checked = false);
  }
});

reportCheckboxes.forEach(cb => {
  cb.addEventListener('change', () => {
    if (cb.checked) {
      optSummary.checked = false;
    } else {
      // If no report checkboxes are selected, auto-select summary
      const anySelected = reportCheckboxes.some(c => c.checked);
      if (!anySelected) {
        optSummary.checked = true;
      }
    }
  });
});

// View toggle logic
viewRichRadio.addEventListener('change', () => {
  if (viewRichRadio.checked) {
    richOutputDiv.style.display = 'block';
    outputPre.style.display = 'none';
  }
});

viewRawRadio.addEventListener('change', () => {
  if (viewRawRadio.checked) {
    richOutputDiv.style.display = 'none';
    outputPre.style.display = 'block';
  }
});

// Get home directory for path shortening
let homeDir = '';

// Shorten path by replacing home dir with ~
function shortenPath(path) {
  if (homeDir && path.startsWith(homeDir)) {
    return '~' + path.slice(homeDir.length);
  }
  return path;
}

// Initialize: get home dir, then restore saved paths with shortened display
(async () => {
  try {
    homeDir = await invoke('get_home_dir');
  } catch (e) {
    console.error('Could not get home dir:', e);
  }

  // Restore saved paths on load (after homeDir is available)
  const savedMkdocs = localStorage.getItem(STORAGE_MKDOCS);
  const savedHelpUrls = localStorage.getItem(STORAGE_HELP_URLS);
  if (savedMkdocs) mkdocsPathInput.value = shortenPath(savedMkdocs);
  if (savedHelpUrls) helpUrlsPathInput.value = shortenPath(savedHelpUrls);
})();

// File browsing
browseMkdocsBtn.addEventListener('click', async () => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'YAML', extensions: ['yml', 'yaml'] }]
    });
    if (selected) {
      mkdocsPathInput.value = shortenPath(selected);
      localStorage.setItem(STORAGE_MKDOCS, selected);
    }
  } catch (err) {
    console.error('Error opening file dialog:', err);
  }
});

browseHelpUrlsBtn.addEventListener('click', async () => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'Header', extensions: ['h'] }]
    });
    if (selected) {
      helpUrlsPathInput.value = shortenPath(selected);
      localStorage.setItem(STORAGE_HELP_URLS, selected);
    }
  } catch (err) {
    console.error('Error opening file dialog:', err);
  }
});

// Convert filesystem path to documentation URL
// e.g., "language-reference-guide/docs/primitive-operators/atop.md"
//    -> "https://dyalog.github.io/documentation/20.0/language-reference-guide/primitive-operators/atop/"
function pathToUrl(path, version) {
  // Remove leading path components before subsite
  let url = path;

  // Remove /docs/ from path
  url = url.replace(/\/docs\//, '/');

  // Remove .md extension
  url = url.replace(/\.md$/, '');

  // Remove /index suffix (directory index pages)
  url = url.replace(/\/index$/, '');

  // Ensure trailing slash for clean URLs
  if (!url.endsWith('/')) {
    url += '/';
  }

  return `${DOCS_BASE}/${version}/${url}`;
}

// Open URL in default browser
async function openUrl(url) {
  try {
    await shellOpen(url);
  } catch (err) {
    // Fallback: open in new window (won't work in Tauri but good for debugging)
    console.error('Failed to open URL:', err);
    window.open(url, '_blank');
  }
}

// Run audit
runAuditBtn.addEventListener('click', async () => {
  // Use full paths from localStorage (display shows shortened versions)
  const mkdocsYaml = localStorage.getItem(STORAGE_MKDOCS);
  const helpUrls = localStorage.getItem(STORAGE_HELP_URLS);

  if (!mkdocsYaml || !helpUrls) {
    alert('Please select both mkdocs.yml and help_urls.h files');
    return;
  }

  // Show spinner immediately
  runAuditBtn.disabled = true;
  runAuditBtn.innerHTML = '<span class="spinner"></span>Running...';

  // Force a repaint before starting async work
  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
  resultsSection.style.display = 'block';
  countsDiv.innerHTML = '';
  outputPre.textContent = '';
  richOutputDiv.innerHTML = '';
  gitInfoEl.textContent = '';

  try {
    const result = await invoke('run_audit', {
      options: {
        mkdocs_yaml: mkdocsYaml,
        help_urls: helpUrls,
        nav_missing: optNavMissing.checked,
        ghost: optGhost.checked,
        help_missing: optHelpMissing.checked,
        broken_links: optBrokenLinks.checked,
        missing_images: optMissingImages.checked,
        orphan_images: optOrphanImages.checked,
        footnotes: optFootnotes.checked,
        summary: optSummary.checked,
        exclude: excludeInput.value.toLowerCase()
      }
    });

    if (result.success) {
      displayCounts(result.counts);
      outputPre.textContent = result.output || '(no output)';
      displayRichOutput(result.items, result.counts, versionInput.value, optSummary.checked);
      if (result.git_info) {
        gitInfoEl.textContent = `${result.git_info.branch} @ ${result.git_info.hash_short}`;
      }
    } else {
      countsDiv.innerHTML = `<div class="error">Error: ${result.error}</div>`;
      outputPre.textContent = '';
      richOutputDiv.innerHTML = '';
    }
  } catch (err) {
    countsDiv.innerHTML = `<div class="error">Error: ${err}</div>`;
    outputPre.textContent = '';
    richOutputDiv.innerHTML = '';
  } finally {
    runAuditBtn.disabled = false;
    runAuditBtn.innerHTML = 'Run Audit';
  }
});

function displayCounts(counts) {
  const items = [
    { key: 'nav_missing', label: 'Nav Missing', checkbox: optNavMissing },
    { key: 'ghost', label: 'Ghost Files', checkbox: optGhost },
    { key: 'help_missing', label: 'Help Missing', checkbox: optHelpMissing },
    { key: 'broken_links', label: 'Broken Links', checkbox: optBrokenLinks },
    { key: 'missing_images', label: 'Missing Images', checkbox: optMissingImages },
    { key: 'orphan_images', label: 'Orphan Images', checkbox: optOrphanImages },
    { key: 'footnotes', label: 'Footnotes', checkbox: optFootnotes, isInfo: true },
  ];

  // Determine which items to show based on checkbox state
  // If no specific checkboxes are selected (summary mode or default), show all except info-only items
  const anySpecificSelected = reportCheckboxes.some(cb => cb.checked);

  const visibleItems = items.filter(item => {
    if (!anySpecificSelected) {
      // Show all issue types when no specific selection (summary mode), but not info-only items
      return !item.isInfo;
    }
    // Show only selected checkboxes
    return item.checkbox.checked;
  });

  const isClickable = visibleItems.length > 1;

  countsDiv.innerHTML = visibleItems
    .filter(item => counts[item.key] !== undefined)
    .map(item => {
      const value = counts[item.key];
      const hasIssues = !item.isInfo && value > 0;
      const clickableClass = isClickable ? 'clickable' : '';
      return `
        <div class="count-item ${hasIssues ? 'has-issues' : ''} ${clickableClass}" data-section="${item.key}">
          <span class="number">${value}</span>
          <span class="label">${item.label}</span>
        </div>
      `;
    })
    .join('');

  // Add total only if more than one item is visible
  if (visibleItems.length > 1) {
    countsDiv.innerHTML += `
      <div class="count-item ${counts.total > 0 ? 'has-issues' : ''}">
        <span class="number">${counts.total}</span>
        <span class="label">Total</span>
      </div>
    `;
  }

  // Add click handlers for scrolling to sections
  if (isClickable) {
    countsDiv.querySelectorAll('.count-item[data-section]').forEach(item => {
      item.addEventListener('click', () => {
        const sectionKey = item.dataset.section;
        const sectionEl = document.querySelector(`.issue-section[data-section="${sectionKey}"]`);
        if (sectionEl) {
          sectionEl.scrollIntoView({ behavior: 'smooth', block: 'start' });
        }
      });
    });
  }
}

function displayRichOutput(items, counts, version, summaryOnly) {
  let html = '';

  // In summary mode, don't show detailed lists
  if (summaryOnly) {
    richOutputDiv.innerHTML = '';
    return;
  }

  // Nav missing
  if (items.nav_missing && items.nav_missing.length > 0) {
    html += renderPlainSection('Missing nav entries', items.nav_missing, 'nav_missing');
  }

  // Ghost files
  if (items.ghost && items.ghost.length > 0) {
    html += renderPlainSection('Ghost files (orphans)', items.ghost, 'ghost');
  }

  // Help missing
  if (items.help_missing && items.help_missing.length > 0) {
    html += renderPlainSection('Missing help URLs', items.help_missing, 'help_missing');
  }

  // Broken links - these get clickable links to the source page
  if (items.broken_links && items.broken_links.length > 0) {
    html += renderBrokenLinksSection('Broken links', items.broken_links, version, 'broken_links');
  }

  // Missing images
  if (items.missing_images && items.missing_images.length > 0) {
    html += renderBrokenImagesSection('Missing images', items.missing_images, version, 'missing_images');
  }

  // Orphan images
  if (items.orphan_images && items.orphan_images.length > 0) {
    html += renderPlainSection('Orphan images', items.orphan_images, 'orphan_images');
  }

  // Footnotes - clickable to open in editor
  if (items.footnotes && items.footnotes.length > 0) {
    html += renderClickableFileSection('Pages with footnotes', items.footnotes, 'footnotes');
  }

  if (!html) {
    html = '<div class="issue-section"><em>No issues found</em></div>';
  }

  richOutputDiv.innerHTML = html;

  // Add click handlers for links
  richOutputDiv.querySelectorAll('.issue-link').forEach(link => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const url = link.dataset.url;
      if (url) {
        openUrl(url);
      }
    });
  });

  // Add click handlers for file links (open in editor)
  richOutputDiv.querySelectorAll('.file-link').forEach(link => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      const path = link.dataset.path;
      if (path) {
        openInEditor(path);
      }
    });
  });
}

function renderPlainSection(title, paths, sectionKey) {
  const listItems = paths.map(path => {
    return `<li class="issue-item">${escapeHtml(path)}</li>`;
  }).join('');

  return `
    <div class="issue-section" data-section="${sectionKey}">
      <h3>${escapeHtml(title)} (${paths.length})</h3>
      <ul class="issue-list">${listItems}</ul>
    </div>
  `;
}

function renderClickableFileSection(title, paths, sectionKey) {
  const listItems = paths.map(path => {
    return `<li class="issue-item"><a class="file-link" data-path="${escapeHtml(path)}" title="Open in editor">${escapeHtml(path)}</a></li>`;
  }).join('');

  return `
    <div class="issue-section" data-section="${sectionKey}">
      <h3>${escapeHtml(title)} (${paths.length})</h3>
      <ul class="issue-list">${listItems}</ul>
    </div>
  `;
}

// Open a file in the user's editor
async function openInEditor(relativePath) {
  const mkdocsPath = localStorage.getItem(STORAGE_MKDOCS);
  if (!mkdocsPath) return;

  // Get the parent directory of mkdocs.yml (monorepo root)
  const basePath = mkdocsPath.substring(0, mkdocsPath.lastIndexOf('/'));
  const fullPath = `${basePath}/${relativePath}`;

  try {
    await invoke('open_in_editor', { filePath: fullPath });
  } catch (err) {
    console.error('Failed to open in editor:', err);
  }
}

function renderBrokenLinksSection(title, links, version, sectionKey) {
  const listItems = links.map(bl => {
    const url = pathToUrl(bl.from, version);
    const marker = bl.from_help_url ? '<span class="help-url-marker">H</span>' : '';
    return `<li class="issue-item">${marker}<a class="issue-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(bl.from)}</a><span class="issue-arrow">-></span><span class="issue-target">${escapeHtml(bl.link)}</span></li>`;
  }).join('');

  return `
    <div class="issue-section" data-section="${sectionKey}">
      <h3>${escapeHtml(title)} (${links.length})</h3>
      <ul class="issue-list">${listItems}</ul>
    </div>
  `;
}

function renderBrokenImagesSection(title, images, version, sectionKey) {
  const listItems = images.map(bi => {
    const url = pathToUrl(bi.from, version);
    return `<li class="issue-item"><a class="issue-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(bi.from)}</a><span class="issue-arrow">-></span><span class="issue-target">${escapeHtml(bi.image)}</span></li>`;
  }).join('');

  return `
    <div class="issue-section" data-section="${sectionKey}">
      <h3>${escapeHtml(title)} (${images.length})</h3>
      <ul class="issue-list">${listItems}</ul>
    </div>
  `;
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

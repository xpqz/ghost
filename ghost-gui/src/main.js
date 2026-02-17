const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

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
const gitInfoEl = document.getElementById('git-info');

// Checkboxes
const optNavMissing = document.getElementById('opt-nav-missing');
const optGhost = document.getElementById('opt-ghost');
const optHelpMissing = document.getElementById('opt-help-missing');
const optBrokenLinks = document.getElementById('opt-broken-links');
const optMissingImages = document.getElementById('opt-missing-images');
const optOrphanImages = document.getElementById('opt-orphan-images');
const optFootnotes = document.getElementById('opt-footnotes');
const optHasImages = document.getElementById('opt-has-images');
const optHasLinks = document.getElementById('opt-has-links');
const optSummary = document.getElementById('opt-summary');
const excludeInput = document.getElementById('exclude');

// Tab elements
const tabAudit = document.getElementById('tab-audit');
const tabSearch = document.getElementById('tab-search');
const auditControls = document.getElementById('audit-controls');
const searchControls = document.getElementById('search-controls');

// Search elements
const searchQueryInput = document.getElementById('search-query');
const runSearchBtn = document.getElementById('run-search');
const searchRegexCheck = document.getElementById('search-regex');
const searchCaseSensitiveCheck = document.getElementById('search-case-sensitive');
const searchContextSelect = document.getElementById('search-context');

// Report type checkboxes (not including summary) - audit tab only
const reportCheckboxes = [optNavMissing, optGhost, optHelpMissing, optBrokenLinks, optMissingImages, optOrphanImages];

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

// Tab switching logic
tabAudit.addEventListener('click', () => {
  tabAudit.classList.add('active');
  tabSearch.classList.remove('active');
  auditControls.style.display = 'block';
  searchControls.style.display = 'none';
  resultsSection.style.display = 'none';
});

tabSearch.addEventListener('click', () => {
  tabSearch.classList.add('active');
  tabAudit.classList.remove('active');
  auditControls.style.display = 'none';
  searchControls.style.display = 'block';
  resultsSection.style.display = 'none';
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
        has_images: optHasImages.checked,
        has_links: optHasLinks.checked,
        summary: optSummary.checked,
        exclude: excludeInput.value.toLowerCase()
      }
    });

    if (result.success) {
      displayCounts(result.counts);
      outputPre.textContent = result.output || '(no output)';
      displayRichOutput(result.items, result.counts, optSummary.checked);
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
  ];

  // Determine which items to show based on checkbox state
  const anySpecificSelected = reportCheckboxes.some(cb => cb.checked);

  const visibleItems = anySpecificSelected
    ? items.filter(item => item.checkbox.checked)
    : items; // Show all when in summary mode

  const isClickable = visibleItems.length > 1;

  countsDiv.innerHTML = visibleItems
    .filter(item => counts[item.key] !== undefined)
    .map(item => {
      const value = counts[item.key];
      const hasIssues = value > 0;
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

function displayRichOutput(items, counts, summaryOnly) {
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
    html += renderBrokenLinksSection('Broken links', items.broken_links, 'broken_links');
  }

  // Missing images
  if (items.missing_images && items.missing_images.length > 0) {
    html += renderBrokenImagesSection('Missing images', items.missing_images, 'missing_images');
  }

  // Orphan images
  if (items.orphan_images && items.orphan_images.length > 0) {
    html += renderPlainSection('Orphan images', items.orphan_images, 'orphan_images');
  }

  if (!html) {
    html = '<div class="issue-section"><em>No issues found</em></div>';
  }

  richOutputDiv.innerHTML = html;

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
  const mkdocsYaml = localStorage.getItem(STORAGE_MKDOCS);
  if (!mkdocsYaml) return;

  try {
    await invoke('open_in_editor', { mkdocsYaml, relativePath });
  } catch (err) {
    console.error('Failed to open in editor:', err);
  }
}

function renderBrokenLinksSection(title, links, sectionKey) {
  const listItems = links.map(bl => {
    const marker = bl.from_help_url ? '<span class="help-url-marker">H</span>' : '';
    return `<li class="issue-item">${marker}<a class="file-link" data-path="${escapeHtml(bl.from)}" title="Open in editor">${escapeHtml(bl.from)}</a><span class="issue-arrow">-></span><span class="issue-target">${escapeHtml(bl.link)}</span></li>`;
  }).join('');

  return `
    <div class="issue-section" data-section="${sectionKey}">
      <h3>${escapeHtml(title)} (${links.length})</h3>
      <ul class="issue-list">${listItems}</ul>
    </div>
  `;
}

function renderBrokenImagesSection(title, images, sectionKey) {
  const listItems = images.map(bi => {
    return `<li class="issue-item"><a class="file-link" data-path="${escapeHtml(bi.from)}" title="Open in editor">${escapeHtml(bi.from)}</a><span class="issue-arrow">-></span><span class="issue-target">${escapeHtml(bi.image)}</span></li>`;
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


// Run search
runSearchBtn.addEventListener('click', async () => {
  const query = searchQueryInput.value.trim();
  const hasFilters = optFootnotes.checked || optHasImages.checked || optHasLinks.checked;

  if (!query && !hasFilters) {
    alert('Please enter a search term or select a filter');
    return;
  }

  const mkdocsYaml = localStorage.getItem(STORAGE_MKDOCS);
  if (!mkdocsYaml) {
    alert('Please select an mkdocs.yml file first');
    return;
  }

  // Show spinner
  runSearchBtn.disabled = true;
  runSearchBtn.innerHTML = '<span class="spinner"></span>Searching...';

  // Show results section and clear previous results
  resultsSection.style.display = 'block';
  countsDiv.innerHTML = '';
  outputPre.textContent = '';
  richOutputDiv.innerHTML = '';
  gitInfoEl.textContent = '';

  try {
    const result = await invoke('search_docs', {
      options: {
        mkdocs_yaml: mkdocsYaml,
        query: query,
        is_regex: searchRegexCheck.checked,
        case_sensitive: searchCaseSensitiveCheck.checked,
        context_lines: parseInt(searchContextSelect.value, 10),
        max_results: 500,
        filter_footnotes: optFootnotes.checked,
        filter_has_images: optHasImages.checked,
        filter_has_links: optHasLinks.checked
      }
    });

    displaySearchResults(result);
  } catch (err) {
    richOutputDiv.innerHTML = `<div class="search-error">Error: ${err}</div>`;
  } finally {
    runSearchBtn.disabled = false;
    runSearchBtn.innerHTML = 'Search';
  }
});

// Allow Enter key to trigger search
searchQueryInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    runSearchBtn.click();
  }
});

function displaySearchResults(result) {
  if (!result.success) {
    richOutputDiv.innerHTML = `<div class="search-error">${escapeHtml(result.error)}</div>`;
    return;
  }

  // Display git info
  if (result.git_info) {
    gitInfoEl.textContent = `${result.git_info.branch} @ ${result.git_info.hash_short}`;
  }

  // Summary in counts area
  const truncatedNote = result.truncated ? ' (results truncated)' : '';
  const summaryClass = result.truncated ? 'search-summary truncated' : 'search-summary';
  // Check if this is filter-only (no query, just filters)
  const isFilterOnlyMode = result.results.length > 0 && result.results[0].matches.length === 0;
  const summaryText = isFilterOnlyMode
    ? `Found ${result.results.length} files (${result.files_searched} files searched)${truncatedNote}`
    : `Found ${result.total_matches} matches in ${result.results.length} files (${result.files_searched} files searched)${truncatedNote}`;
  countsDiv.innerHTML = `<div class="${summaryClass}">${summaryText}</div>`;

  if (result.results.length === 0) {
    richOutputDiv.innerHTML = '<em>No matches found</em>';
    return;
  }

  // Check if this is filter-only mode (no matches, just file list)
  const isFilterOnly = result.results.length > 0 && result.results[0].matches.length === 0;

  // Render results grouped by file
  let html = '';
  for (const fileResult of result.results) {
    if (isFilterOnly) {
      // Simple file list for filter-only mode
      html += `
        <div class="search-file-item">
          <a class="file-link" data-path="${escapeHtml(fileResult.file_path)}" title="Click to open in editor">${escapeHtml(fileResult.file_path)}</a>
        </div>
      `;
    } else {
      html += `
        <div class="search-file-group">
          <div class="search-file-header" data-path="${escapeHtml(fileResult.file_path)}" title="Click to open in editor">
            ${escapeHtml(fileResult.file_path)}
            <span class="match-count">(${fileResult.matches.length} match${fileResult.matches.length !== 1 ? 'es' : ''})</span>
          </div>
          <div class="search-file-matches">
      `;

      for (const match of fileResult.matches) {
        html += renderSearchMatch(match, fileResult.file_path);
      }

      html += '</div></div>';
    }
  }

  richOutputDiv.innerHTML = html;

  // Add click handlers for file headers (search with matches)
  richOutputDiv.querySelectorAll('.search-file-header').forEach(header => {
    header.addEventListener('click', () => {
      openInEditor(header.dataset.path);
    });
  });

  // Add click handlers for match items
  richOutputDiv.querySelectorAll('.search-match-item').forEach(item => {
    item.addEventListener('click', () => {
      const path = item.dataset.path;
      const line = item.dataset.line;
      openInEditorAtLine(path, line);
    });
  });

  // Add click handlers for file links (filter-only mode)
  richOutputDiv.querySelectorAll('.search-file-item .file-link').forEach(link => {
    link.addEventListener('click', (e) => {
      e.preventDefault();
      openInEditor(link.dataset.path);
    });
  });
}

function renderSearchMatch(match, filePath) {
  let html = `<div class="search-match-item" data-path="${escapeHtml(filePath)}" data-line="${match.line_number}">`;

  // Context before
  const startLineNum = match.line_number - match.context_before.length;
  for (let i = 0; i < match.context_before.length; i++) {
    const lineNum = startLineNum + i;
    html += `<div class="search-context-line"><span class="line-number">${lineNum}</span>${escapeHtml(match.context_before[i])}</div>`;
  }

  // Matching line with highlight
  const before = match.line_content.substring(0, match.match_start);
  const matched = match.line_content.substring(match.match_start, match.match_end);
  const after = match.line_content.substring(match.match_end);

  html += `<div class="search-match-line"><span class="line-number">${match.line_number}</span>${escapeHtml(before)}<span class="highlight">${escapeHtml(matched)}</span>${escapeHtml(after)}</div>`;

  // Context after
  for (let i = 0; i < match.context_after.length; i++) {
    const lineNum = match.line_number + i + 1;
    html += `<div class="search-context-line"><span class="line-number">${lineNum}</span>${escapeHtml(match.context_after[i])}</div>`;
  }

  html += '</div>';
  return html;
}

// Open file in editor at specific line
async function openInEditorAtLine(relativePath, lineNumber) {
  const mkdocsYaml = localStorage.getItem(STORAGE_MKDOCS);
  if (!mkdocsYaml) return;

  try {
    await invoke('open_in_editor', { mkdocsYaml, relativePath, lineNumber });
  } catch (err) {
    // Fallback: try without line number
    try {
      await invoke('open_in_editor', { mkdocsYaml, relativePath });
    } catch (err2) {
      console.error('Failed to open in editor:', err2);
    }
  }
}

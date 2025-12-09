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

// Checkboxes
const optNavMissing = document.getElementById('opt-nav-missing');
const optGhost = document.getElementById('opt-ghost');
const optHelpMissing = document.getElementById('opt-help-missing');
const optBrokenLinks = document.getElementById('opt-broken-links');
const optMissingImages = document.getElementById('opt-missing-images');
const optOrphanImages = document.getElementById('opt-orphan-images');
const optSummary = document.getElementById('opt-summary');
const excludeInput = document.getElementById('exclude');

// Report type checkboxes (not including summary)
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
    }
  });
});

// Restore saved paths on load
const savedMkdocs = localStorage.getItem(STORAGE_MKDOCS);
const savedHelpUrls = localStorage.getItem(STORAGE_HELP_URLS);
if (savedMkdocs) mkdocsPathInput.value = savedMkdocs;
if (savedHelpUrls) helpUrlsPathInput.value = savedHelpUrls;

// File browsing
browseMkdocsBtn.addEventListener('click', async () => {
  try {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'YAML', extensions: ['yml', 'yaml'] }]
    });
    if (selected) {
      mkdocsPathInput.value = selected;
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
      helpUrlsPathInput.value = selected;
      localStorage.setItem(STORAGE_HELP_URLS, selected);
    }
  } catch (err) {
    console.error('Error opening file dialog:', err);
  }
});

// Run audit
runAuditBtn.addEventListener('click', async () => {
  const mkdocsYaml = mkdocsPathInput.value;
  const helpUrls = helpUrlsPathInput.value;

  if (!mkdocsYaml || !helpUrls) {
    alert('Please select both mkdocs.yml and help_urls.h files');
    return;
  }

  runAuditBtn.disabled = true;
  runAuditBtn.textContent = 'Running...';
  resultsSection.style.display = 'block';
  countsDiv.innerHTML = '';
  outputPre.textContent = 'Running audit...';

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
        summary: optSummary.checked,
        exclude: excludeInput.value.toLowerCase()
      }
    });

    if (result.success) {
      displayCounts(result.counts);
      outputPre.textContent = result.output || '(no output)';
    } else {
      countsDiv.innerHTML = `<div class="error">Error: ${result.error}</div>`;
      outputPre.textContent = '';
    }
  } catch (err) {
    countsDiv.innerHTML = `<div class="error">Error: ${err}</div>`;
    outputPre.textContent = '';
  } finally {
    runAuditBtn.disabled = false;
    runAuditBtn.textContent = 'Run Audit';
  }
});

function displayCounts(counts) {
  const items = [
    { key: 'nav_missing', label: 'Nav Missing' },
    { key: 'ghost', label: 'Ghost Files' },
    { key: 'help_missing', label: 'Help Missing' },
    { key: 'broken_links', label: 'Broken Links' },
    { key: 'missing_images', label: 'Missing Images' },
    { key: 'orphan_images', label: 'Orphan Images' },
  ];

  countsDiv.innerHTML = items
    .filter(item => counts[item.key] !== undefined)
    .map(item => {
      const value = counts[item.key];
      const hasIssues = value > 0;
      return `
        <div class="count-item ${hasIssues ? 'has-issues' : ''}">
          <span class="number">${value}</span>
          <span class="label">${item.label}</span>
        </div>
      `;
    })
    .join('');

  // Add total
  countsDiv.innerHTML += `
    <div class="count-item ${counts.total > 0 ? 'has-issues' : ''}">
      <span class="number">${counts.total}</span>
      <span class="label">Total</span>
    </div>
  `;
}

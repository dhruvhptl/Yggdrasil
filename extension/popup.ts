// Popup controller for the Yggdrasil Chrome extension.

const API_BASE = 'http://127.0.0.1:1421';
const API_KEY = 'ygg-local-dev';
const QUEUE_KEY = 'ygg_job_queue';
const DRAFT_KEY = 'ygg_form_draft';

type PopupState = 'idle' | 'submitting' | 'success' | 'error' | 'offline';

interface PageContext {
  company: string;
  position: string;
}

interface TailoredProject {
  name: string;
  description: string | null;
  github_url: string | null;
  score: number;
}

interface JobResponse {
  id: string;
  tailored_projects: TailoredProject[];
}

interface QueuedJob {
  company: string;
  position: string;
  location: string;
  source: string;
  link: string;
  season: string;
  jobDescription: string;
  queuedAt: string;
}

interface FormDraft {
  company: string;
  position: string;
  location: string;
  source: string;
  link: string;
  season: string;
  jobDescription: string;
}

// ─── Runtime state ────────────────────────────────────────────────────────────

let currentJobId: string | null = null;
const ratings: Record<string, number> = { location: 0, alignment: 0, salary: 0, role: 0 };

let activeTab: 'job' | 'library' = 'job';

// ─── Library types ────────────────────────────────────────────────────────────

interface LibraryEntry {
  id: string;
  title: string;
  url: string;
  type: string;
  tags: string[];
  user_notes: string | null;
  created_at: string;
}

interface LibraryQueuedItem {
  url: string;
  title: string;
  note: string;
  queued_at: string;
}

const LIBRARY_QUEUE_KEY = 'ygg_library_queue';

// ─── Tag-input state ──────────────────────────────────────────────────────────

interface TagFieldState {
  chips: string[];
  presets: string[]; // grows with custom "Other" additions this session
}

const tagFields: Record<string, TagFieldState> = {
  location: { chips: [], presets: [] },
  source: { chips: [], presets: [] },
};

function setTagPresets(field: string, presets: string[]): void {
  // Merge with any custom chips already in state so they remain selectable
  const extra = tagFields[field].chips.filter((c) => !presets.includes(c));
  tagFields[field].presets = [...presets, ...extra];
  renderTagInput(field);
}

function getTagValue(field: string): string {
  return tagFields[field].chips.join(', ');
}

function setTagValue(field: string, value: string): void {
  tagFields[field].chips = value
    ? value
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean)
    : [];
  renderTagInput(field);
}

function addTagChip(field: string, rawValue: string): void {
  const value = rawValue.trim();
  if (!value || tagFields[field].chips.includes(value)) return;
  tagFields[field].chips.push(value);
  // Persist custom values in the preset list for the session
  if (!tagFields[field].presets.includes(value)) {
    tagFields[field].presets.push(value);
    tagFields[field].presets.sort();
  }
  renderTagInput(field);
  void saveDraft();
}

function removeTagChip(field: string, value: string): void {
  tagFields[field].chips = tagFields[field].chips.filter((c) => c !== value);
  renderTagInput(field);
  void saveDraft();
}

function renderTagInput(field: string): void {
  const state = tagFields[field];

  // ── Chips ──
  const chipsEl = document.getElementById(`chips-${field}`)!;
  chipsEl.innerHTML = '';
  state.chips.forEach((chip) => {
    const span = document.createElement('span');
    span.className = 'chip';
    span.innerHTML =
      `${escHtml(chip)}<button type="button" class="chip-remove" ` +
      `data-field="${field}" data-chip="${escHtml(chip)}" aria-label="Remove">&times;</button>`;
    chipsEl.appendChild(span);
  });

  // ── Preset pills (exclude already-selected chips) ──
  const presetsEl = document.getElementById(`presets-${field}`)!;
  presetsEl.innerHTML = '';
  state.presets
    .filter((p) => !state.chips.includes(p))
    .forEach((preset) => {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'preset-pill';
      btn.textContent = preset;
      btn.dataset.field = field;
      btn.dataset.preset = preset;
      presetsEl.appendChild(btn);
    });

  // Always append "Other" last
  const otherBtn = document.createElement('button');
  otherBtn.type = 'button';
  otherBtn.className = 'preset-pill preset-other';
  otherBtn.textContent = 'Other';
  otherBtn.dataset.field = field;
  otherBtn.dataset.preset = '__other__';
  presetsEl.appendChild(otherBtn);
}

function showOtherInput(field: string, visible: boolean): void {
  const row = document.getElementById(`other-${field}`)!;
  row.style.display = visible ? 'flex' : 'none';
  if (visible) {
    (document.getElementById(`other-input-${field}`) as HTMLInputElement).focus();
  }
}

function commitOtherValue(field: string): void {
  const input = document.getElementById(`other-input-${field}`) as HTMLInputElement;
  const val = input.value.trim();
  if (val) {
    addTagChip(field, val);
    input.value = '';
  }
  showOtherInput(field, false);
}

// ─── Season generation ────────────────────────────────────────────────────────

function generateSeasons(): string[] {
  const now = new Date();
  const month = now.getMonth() + 1;
  let year = now.getFullYear();
  const order = ['Winter', 'Spring', 'Fall'] as const;
  let si = month <= 4 ? 0 : month <= 8 ? 1 : 2;
  const result: string[] = [];
  for (let i = 0; i < 5; i++) {
    result.push(`${order[si]} ${year}`);
    si = (si + 1) % 3;
    if (si === 0) year++;
  }
  return result;
}

// ─── Draft persistence ────────────────────────────────────────────────────────

async function saveDraft(): Promise<void> {
  const draft: FormDraft = {
    company: (document.getElementById('company') as HTMLInputElement).value,
    position: (document.getElementById('position') as HTMLInputElement).value,
    location: getTagValue('location'),
    source: getTagValue('source'),
    link: (document.getElementById('link') as HTMLInputElement).value,
    season: (document.getElementById('season') as HTMLSelectElement).value,
    jobDescription: (document.getElementById('job-description') as HTMLTextAreaElement).value,
  };
  await chrome.storage.local.set({ [DRAFT_KEY]: draft });
}

async function loadDraft(): Promise<FormDraft | null> {
  const data = await chrome.storage.local.get(DRAFT_KEY);
  return (data[DRAFT_KEY] as FormDraft) ?? null;
}

async function clearDraft(): Promise<void> {
  await chrome.storage.local.remove(DRAFT_KEY);
}

function applyDraft(draft: FormDraft): void {
  (document.getElementById('company') as HTMLInputElement).value = draft.company;
  (document.getElementById('position') as HTMLInputElement).value = draft.position;
  setTagValue('location', draft.location ?? '');
  setTagValue('source', draft.source ?? '');
  (document.getElementById('link') as HTMLInputElement).value = draft.link;
  const seasonEl = document.getElementById('season') as HTMLSelectElement;
  if (draft.season) {
    for (let i = 0; i < seasonEl.options.length; i++) {
      if (seasonEl.options[i].value === draft.season) {
        seasonEl.selectedIndex = i;
        break;
      }
    }
  }
  (document.getElementById('job-description') as HTMLTextAreaElement).value = draft.jobDescription;
}

// ─── API helpers ──────────────────────────────────────────────────────────────

async function checkHealth(): Promise<boolean> {
  try {
    const controller = new AbortController();
    const t = setTimeout(() => controller.abort(), 2500);
    const res = await fetch(`${API_BASE}/api/health`, { signal: controller.signal });
    clearTimeout(t);
    return res.ok;
  } catch {
    return false;
  }
}

async function fetchMeta(): Promise<{ sources: string[]; locations: string[] }> {
  try {
    const controller = new AbortController();
    const t = setTimeout(() => controller.abort(), 3000);
    const res = await fetch(`${API_BASE}/api/jobs/meta`, {
      headers: { 'X-Ygg-Key': API_KEY },
      signal: controller.signal,
    });
    clearTimeout(t);
    if (!res.ok) return { sources: [], locations: [] };
    return res.json() as Promise<{ sources: string[]; locations: string[] }>;
  } catch {
    return { sources: [], locations: [] };
  }
}

async function submitJob(payload: QueuedJob): Promise<JobResponse> {
  const res = await fetch(`${API_BASE}/api/jobs`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
    body: JSON.stringify(payload),
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
    throw new Error((body as { error?: string }).error ?? `HTTP ${res.status}`);
  }
  return res.json() as Promise<JobResponse>;
}

async function patchJob(id: string, body: object): Promise<void> {
  const res = await fetch(`${API_BASE}/api/jobs/${id}`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const data = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
    throw new Error((data as { error?: string }).error ?? `HTTP ${res.status}`);
  }
}

// ─── Queue helpers ────────────────────────────────────────────────────────────

async function getQueue(): Promise<QueuedJob[]> {
  const data = await chrome.storage.local.get(QUEUE_KEY);
  return (data[QUEUE_KEY] as QueuedJob[]) ?? [];
}

async function saveQueue(queue: QueuedJob[]): Promise<void> {
  await chrome.storage.local.set({ [QUEUE_KEY]: queue });
  await chrome.action.setBadgeText({ text: queue.length > 0 ? String(queue.length) : '' });
  if (queue.length > 0) await chrome.action.setBadgeBackgroundColor({ color: '#4f98a3' });
}

async function enqueue(job: QueuedJob): Promise<void> {
  const queue = await getQueue();
  queue.push(job);
  await saveQueue(queue);
}

async function flushQueue(): Promise<void> {
  const queue = await getQueue();
  if (queue.length === 0) return;
  const remaining: QueuedJob[] = [];
  for (const job of queue) {
    try { await submitJob(job); } catch { remaining.push(job); }
  }
  await saveQueue(remaining);
}

// ─── DOM helpers ──────────────────────────────────────────────────────────────

function el<T extends HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

function showState(state: PopupState): void {
  for (const v of ['idle', 'submitting', 'success', 'error', 'offline'] as PopupState[]) {
    el(`view-${v}`).style.display =
      v === state ? (v === 'submitting' ? 'flex' : 'block') : 'none';
  }
}

function escHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ─── Ratings UI ───────────────────────────────────────────────────────────────

function updateDotsForField(field: string): void {
  document.querySelectorAll<HTMLButtonElement>(`.dot-btn[data-field="${field}"]`).forEach((btn) => {
    btn.classList.toggle('active', parseInt(btn.dataset.value!, 10) <= ratings[field]);
  });
}

function resetRatings(): void {
  Object.keys(ratings).forEach((k) => (ratings[k] = 0));
  Object.keys(ratings).forEach(updateDotsForField);
}

// ─── Success screen ───────────────────────────────────────────────────────────

function showSuccess(jobId: string, projects: TailoredProject[]): void {
  currentJobId = jobId;
  resetRatings();
  el('status-buttons').style.display = 'flex';
  el('ratings-section').style.display = 'none';
  el('status-result').style.display = 'none';
  el('ratings-error').style.display = 'none';

  const listEl = el('projects-list');
  listEl.innerHTML = '';
  if (projects.length === 0) {
    listEl.innerHTML =
      '<p style="color:#555;font-size:12px;margin-top:4px">No matching resume projects found.</p>';
  } else {
    projects.forEach((p) => {
      const score = Math.round(p.score * 100);
      const item = document.createElement('div');
      item.className = 'project-item';
      const nameHtml = p.github_url
        ? `<a href="${p.github_url}" target="_blank" class="project-name">${escHtml(p.name)}</a>`
        : `<span class="project-name">${escHtml(p.name)}</span>`;
      item.innerHTML = `
        <div class="project-header">${nameHtml}<span class="score-pill">${score}%</span></div>
        ${p.description ? `<p class="project-desc">${escHtml(p.description)}</p>` : ''}
      `;
      listEl.appendChild(item);
    });
  }
  showState('success');
}

// ─── Status action handlers ───────────────────────────────────────────────────

async function handleMarkSaved(): Promise<void> {
  if (!currentJobId) return;
  const btn = el<HTMLButtonElement>('btn-mark-saved');
  btn.disabled = true;
  btn.textContent = '…';
  try {
    await patchJob(currentJobId, { status: 'saved' });
    el('status-buttons').style.display = 'none';
    el('ratings-section').style.display = 'none';
    const r = el('status-result');
    r.textContent = '✓ Marked as Saved';
    r.style.color = '#4caf7d';
    r.style.display = 'block';
  } catch (err) {
    btn.disabled = false;
    btn.textContent = 'Mark Saved';
    const e = el('ratings-error');
    e.textContent = err instanceof Error ? err.message : 'Failed to update';
    e.style.display = 'block';
  }
}

async function handleMarkApplied(): Promise<void> {
  el('status-buttons').style.display = 'none';
  const r = el('ratings-section');
  r.style.display = 'block';
  el<HTMLButtonElement>('save-ratings-btn').disabled = false;
  el('save-ratings-btn').textContent = 'Save Ratings';
  el('ratings-error').style.display = 'none';
}

async function handleSaveRatings(): Promise<void> {
  if (!currentJobId) return;
  const btn = el<HTMLButtonElement>('save-ratings-btn');
  btn.disabled = true;
  btn.textContent = '…';
  try {
    await patchJob(currentJobId, {
      status: 'applied',
      locationRating: ratings.location || null,
      alignmentRating: ratings.alignment || null,
      salaryRating: ratings.salary || null,
      roleRating: ratings.role || null,
    });
    el('ratings-section').style.display = 'none';
    const r = el('status-result');
    r.textContent = '✓ Marked as Applied';
    r.style.color = '#4f98a3';
    r.style.display = 'block';
  } catch (err) {
    btn.disabled = false;
    btn.textContent = 'Save Ratings';
    const e = el('ratings-error');
    e.textContent = err instanceof Error ? err.message : 'Failed to update';
    e.style.display = 'block';
  }
}

// ─── Form collection + reset ──────────────────────────────────────────────────

function collectForm(): QueuedJob {
  return {
    company: (document.getElementById('company') as HTMLInputElement).value.trim(),
    position: (document.getElementById('position') as HTMLInputElement).value.trim(),
    location: getTagValue('location'),
    source: getTagValue('source'),
    link: (document.getElementById('link') as HTMLInputElement).value.trim(),
    season: (document.getElementById('season') as HTMLSelectElement).value,
    jobDescription: (document.getElementById('job-description') as HTMLTextAreaElement).value.trim(),
    queuedAt: new Date().toISOString(),
  };
}

function resetForm(): void {
  (document.getElementById('company') as HTMLInputElement).value = '';
  (document.getElementById('position') as HTMLInputElement).value = '';
  setTagValue('location', '');
  setTagValue('source', '');
  (document.getElementById('link') as HTMLInputElement).value = '';
  (document.getElementById('job-description') as HTMLTextAreaElement).value = '';
  showOtherInput('location', false);
  showOtherInput('source', false);
}

// ─── Submit handler ───────────────────────────────────────────────────────────

async function handleSubmit(): Promise<void> {
  showState('submitting');
  const payload = collectForm();
  if (!payload.company || !payload.position) {
    el('error-message').textContent = 'Company and position are required.';
    showState('error');
    return;
  }
  const online = await checkHealth();
  if (!online) {
    await enqueue(payload);
    showState('offline');
    return;
  }
  try {
    const data = await submitJob(payload);
    await clearDraft();
    showSuccess(data.id, data.tailored_projects);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    if (msg.startsWith('HTTP 4')) {
      el('error-message').textContent = msg;
      showState('error');
    } else {
      await enqueue(payload);
      showState('offline');
    }
  }
}

// ─── Tab switching ────────────────────────────────────────────────────────────

function switchTab(tab: 'job' | 'library'): void {
  activeTab = tab;
  el('tab-job').classList.toggle('tab-active', tab === 'job');
  el('tab-library').classList.toggle('tab-active', tab === 'library');
  el('view-job').style.display = tab === 'job' ? 'block' : 'none';
  el('view-library').style.display = tab === 'library' ? 'block' : 'none';
  if (tab === 'library') void loadLibraryBrowse();
}

// ─── Library helpers ──────────────────────────────────────────────────────────

function extractDomain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

async function getLibraryQueue(): Promise<LibraryQueuedItem[]> {
  const data = await chrome.storage.local.get(LIBRARY_QUEUE_KEY);
  return (data[LIBRARY_QUEUE_KEY] as LibraryQueuedItem[]) ?? [];
}

async function saveLibraryQueue(items: LibraryQueuedItem[]): Promise<void> {
  await chrome.storage.local.set({ [LIBRARY_QUEUE_KEY]: items });
  const jobQ = await getQueue();
  const total = items.length + jobQ.length;
  await chrome.action.setBadgeText({ text: total > 0 ? String(total) : '' });
  if (total > 0) await chrome.action.setBadgeBackgroundColor({ color: '#4f98a3' });
}

async function patchLibraryNote(id: string, note: string): Promise<void> {
  await fetch(`${API_BASE}/api/library/${id}`, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
    body: JSON.stringify({ note }),
  });
}

// ─── Library rendering ────────────────────────────────────────────────────────

function renderLibraryRow(r: LibraryEntry): HTMLElement {
  const row = document.createElement('div');
  row.className = 'lib-row';

  const domain = extractDomain(r.url);
  const displayTags = r.tags.slice(0, 3);
  const extraTags = r.tags.length > 3 ? r.tags.length - 3 : 0;

  const main = document.createElement('div');
  main.className = 'lib-row-main';
  main.innerHTML = `
    <div class="lib-row-title">${escHtml(r.title)}</div>
    <div class="lib-row-domain">${escHtml(domain)}</div>
    ${r.tags.length > 0 ? `<div class="lib-row-tags">
      ${displayTags.map((t) => `<span class="lib-tag">${escHtml(t)}</span>`).join('')}
      ${extraTags > 0 ? `<span class="lib-tag-more">+${extraTags} more</span>` : ''}
    </div>` : ''}
  `;
  main.addEventListener('click', () => { void chrome.tabs.create({ url: r.url }); });

  const noteBtn = document.createElement('button');
  noteBtn.className = 'lib-note-btn';
  noteBtn.title = 'Edit note';
  noteBtn.textContent = '✎';

  const noteEditor = document.createElement('div');
  noteEditor.style.cssText = 'display:none; flex-direction:column; width:100%; padding:6px 0 2px';

  const noteTA = document.createElement('textarea');
  noteTA.className = 'lib-note-textarea';
  noteTA.value = r.user_notes ?? '';

  const noteActions = document.createElement('div');
  noteActions.style.cssText = 'display:flex; gap:6px; margin-top:4px; align-items:center';

  const noteSave = document.createElement('button');
  noteSave.className = 'btn btn-primary';
  noteSave.style.cssText = 'font-size:11px; padding:4px 10px; width:auto';
  noteSave.textContent = 'Save';

  const noteCancel = document.createElement('a');
  noteCancel.href = '#';
  noteCancel.className = 'text-link';
  noteCancel.textContent = 'Cancel';

  noteActions.appendChild(noteSave);
  noteActions.appendChild(noteCancel);
  noteEditor.appendChild(noteTA);
  noteEditor.appendChild(noteActions);

  noteBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const open = noteEditor.style.display !== 'none';
    noteEditor.style.display = open ? 'none' : 'flex';
    if (!open) noteTA.focus();
  });

  noteSave.addEventListener('click', () => {
    void patchLibraryNote(r.id, noteTA.value);
    noteEditor.style.display = 'none';
  });

  noteCancel.addEventListener('click', (e) => {
    e.preventDefault();
    noteEditor.style.display = 'none';
  });

  const wrapper = document.createElement('div');
  wrapper.style.cssText = 'display:flex; flex-direction:column; flex:1; min-width:0';
  wrapper.appendChild(main);
  wrapper.appendChild(noteEditor);

  row.appendChild(wrapper);
  if (r.id) row.appendChild(noteBtn);
  return row;
}

function renderLibraryList(resources: LibraryEntry[]): void {
  const listEl = el('lib-list');
  listEl.innerHTML = '';
  el('lib-empty').style.display = resources.length === 0 ? 'block' : 'none';
  resources.forEach((r) => listEl.appendChild(renderLibraryRow(r)));
}

async function loadLibraryBrowse(): Promise<void> {
  el('lib-offline-banner').style.display = 'none';
  await fetchAndRenderLibrary();
}

async function fetchAndRenderLibrary(q?: string): Promise<void> {
  const params = new URLSearchParams({ limit: '20' });
  if (q) params.set('q', q);

  try {
    const res = await fetch(`${API_BASE}/api/library?${params.toString()}`, {
      headers: { 'X-Ygg-Key': API_KEY },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = (await res.json()) as { resources: LibraryEntry[] };
    renderLibraryList(data.resources);
  } catch {
    if (!q) {
      const queued = await getLibraryQueue();
      const fakeEntries: LibraryEntry[] = queued.map((item) => ({
        id: '',
        title: item.title || item.url,
        url: item.url,
        type: 'webpage',
        tags: [],
        user_notes: item.note || null,
        created_at: item.queued_at,
      }));
      renderLibraryList(fakeEntries);
      el('lib-offline-banner').style.display = 'block';
    }
  }
}

// ─── Library save form ────────────────────────────────────────────────────────

function showSaveForm(visible: boolean): void {
  el('lib-save-form').style.display = visible ? 'block' : 'none';
  el('lib-save-page').style.display = visible ? 'none' : 'inline-block';
  if (!visible) {
    (el('lib-save-title') as HTMLInputElement).value = '';
    (el('lib-save-note') as HTMLTextAreaElement).value = '';
    el('lib-save-status').style.display = 'none';
    el('lib-save-status').textContent = '';
    (el<HTMLButtonElement>('lib-save-submit')).disabled = false;
    (el<HTMLButtonElement>('lib-save-submit')).textContent = 'Save';
  }
}

function showSaveStatus(msg: string, color: string): void {
  const s = el('lib-save-status');
  s.textContent = msg;
  s.style.color = color;
  s.style.display = 'block';
}

async function handleSavePage(): Promise<void> {
  showSaveForm(true);
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (tab?.url && !tab.url.startsWith('chrome://')) {
      (el('lib-save-title') as HTMLInputElement).value = tab.title ?? tab.url;
    }
  } catch { /* activeTab unavailable */ }
}

async function handleLibrarySave(): Promise<void> {
  const btn = el<HTMLButtonElement>('lib-save-submit');
  btn.disabled = true;
  btn.textContent = '…';

  let tab: chrome.tabs.Tab | undefined;
  try {
    [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  } catch { /* noop */ }

  const url = tab?.url ?? '';
  if (!url || url.startsWith('chrome://')) {
    btn.disabled = false;
    btn.textContent = 'Save';
    showSaveStatus('Cannot save this page.', '#e05252');
    return;
  }

  const title = (el('lib-save-title') as HTMLInputElement).value.trim();
  const note = (el('lib-save-note') as HTMLTextAreaElement).value.trim();

  const online = await checkHealth();
  if (!online) {
    const queue = await getLibraryQueue();
    queue.push({ url, title, note, queued_at: new Date().toISOString() });
    await saveLibraryQueue(queue);
    showSaveStatus('Queued — syncs when Yggdrasil opens', '#4f98a3');
    setTimeout(() => {
      showSaveForm(false);
      void fetchAndRenderLibrary();
      el('lib-offline-banner').style.display = 'block';
    }, 1000);
    return;
  }

  try {
    const res = await fetch(`${API_BASE}/api/library`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
      body: JSON.stringify({ url, title, note }),
    });
    if (!res.ok) {
      const data = await res.json().catch(() => ({ error: `HTTP ${res.status}` }));
      throw new Error((data as { error?: string }).error ?? `HTTP ${res.status}`);
    }
    showSaveStatus('✓ Saved', '#4caf7d');
    setTimeout(() => {
      showSaveForm(false);
      void fetchAndRenderLibrary();
    }, 700);
  } catch (err) {
    btn.disabled = false;
    btn.textContent = 'Save';
    showSaveStatus(err instanceof Error ? err.message : 'Save failed', '#e05252');
  }
}

// ─── Library search ───────────────────────────────────────────────────────────

let searchDebounceTimer: ReturnType<typeof setTimeout> | null = null;

function handleLibrarySearch(q: string): void {
  el('lib-search-clear').style.display = q ? 'inline-block' : 'none';
  if (searchDebounceTimer !== null) clearTimeout(searchDebounceTimer);
  if (!q.trim()) {
    void loadLibraryBrowse();
    return;
  }
  searchDebounceTimer = setTimeout(() => void fetchAndRenderLibrary(q.trim()), 300);
}

// ─── Init ─────────────────────────────────────────────────────────────────────

(async () => {
  // Season options
  const seasonEl = document.getElementById('season') as HTMLSelectElement;
  generateSeasons().forEach((s) => {
    const opt = document.createElement('option');
    opt.value = s;
    opt.textContent = s;
    seasonEl.appendChild(opt);
  });

  // Health + meta in parallel (non-blocking — update UI when each resolves)
  const [healthResult, metaResult] = await Promise.allSettled([checkHealth(), fetchMeta()]);
  const healthy = healthResult.status === 'fulfilled' && healthResult.value;
  const meta =
    metaResult.status === 'fulfilled'
      ? metaResult.value
      : { sources: [], locations: [] };

  const dot = el('health-dot');
  dot.className = `dot ${healthy ? 'dot-green' : 'dot-red'}`;
  dot.title = healthy ? 'Yggdrasil is running' : 'Yggdrasil is not running';

  // Populate tag presets from real DB data (empty arrays if offline)
  setTagPresets('location', meta.locations);
  setTagPresets('source', meta.sources);

  if (healthy) void flushQueue();

  // Queue badge
  getQueue().then((q) => {
    if (q.length > 0) {
      const qEl = el('queue-count');
      qEl.textContent = `${q.length} job${q.length !== 1 ? 's' : ''} queued`;
      qEl.style.display = 'block';
    }
  });

  // Draft takes priority over content-script auto-fill
  const draft = await loadDraft();
  if (draft) {
    applyDraft(draft);
  } else {
    try {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (tab.id && tab.url && !tab.url.startsWith('chrome://')) {
        (document.getElementById('link') as HTMLInputElement).value = tab.url;
        try {
          const ctx = (await chrome.tabs.sendMessage(tab.id, {
            type: 'GET_PAGE_CONTEXT',
          })) as PageContext;
          if (ctx.company)
            (document.getElementById('company') as HTMLInputElement).value = ctx.company;
          if (ctx.position)
            (document.getElementById('position') as HTMLInputElement).value = ctx.position;
        } catch {
          if (tab.title) {
            const p = parseTitle(tab.title);
            if (p.company)
              (document.getElementById('company') as HTMLInputElement).value = p.company;
            if (p.position)
              (document.getElementById('position') as HTMLInputElement).value = p.position;
          }
        }
      }
    } catch {
      // activeTab unavailable
    }
  }

  // Save draft on native form-field changes
  el('job-form').addEventListener('input', saveDraft);
  el('job-form').addEventListener('change', saveDraft);

  // Tag input event delegation (chip remove + preset pills)
  document.addEventListener('click', (e) => {
    const target = e.target as HTMLElement;

    if (target.classList.contains('chip-remove')) {
      removeTagChip(target.dataset.field!, target.dataset.chip!);
      return;
    }

    if (target.classList.contains('preset-pill')) {
      const field = target.dataset.field!;
      const preset = target.dataset.preset!;
      if (preset === '__other__') {
        const row = document.getElementById(`other-${field}`)!;
        showOtherInput(field, row.style.display === 'none' || row.style.display === '');
      } else {
        addTagChip(field, preset);
      }
    }
  });

  // "Other" Add buttons and Enter key
  for (const field of ['location', 'source']) {
    el<HTMLButtonElement>(`other-add-${field}`).addEventListener('click', () =>
      commitOtherValue(field),
    );
    el<HTMLInputElement>(`other-input-${field}`).addEventListener('keydown', (e) => {
      if (e.key === 'Enter') { e.preventDefault(); commitOtherValue(field); }
      if (e.key === 'Escape') showOtherInput(field, false);
    });
  }

  // Form submit
  el('job-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    await handleSubmit();
  });

  // Success screen
  el('btn-mark-saved').addEventListener('click', () => void handleMarkSaved());
  el('btn-mark-applied').addEventListener('click', () => void handleMarkApplied());
  el('save-ratings-btn').addEventListener('click', () => void handleSaveRatings());

  // Rating dots
  document.querySelectorAll<HTMLButtonElement>('.dot-btn').forEach((btn) => {
    btn.addEventListener('click', () => {
      const field = btn.dataset.field!;
      ratings[field] = parseInt(btn.dataset.value!, 10);
      updateDotsForField(field);
    });
  });

  // Navigation links
  el('add-another').addEventListener('click', (e) => {
    e.preventDefault();
    resetForm();
    void clearDraft();
    showState('idle');
  });
  el('error-retry').addEventListener('click', () => showState('idle'));
  el('offline-another').addEventListener('click', () => {
    resetForm();
    void clearDraft();
    showState('idle');
  });
  el('open-in-app').addEventListener('click', (e) => e.preventDefault());

  // Tab switching
  el('tab-job').addEventListener('click', () => switchTab('job'));
  el('tab-library').addEventListener('click', () => switchTab('library'));

  // Library save form
  el('lib-save-page').addEventListener('click', () => void handleSavePage());
  el('lib-save-submit').addEventListener('click', () => void handleLibrarySave());
  el('lib-save-cancel').addEventListener('click', (e) => {
    e.preventDefault();
    showSaveForm(false);
  });

  // Library search
  el<HTMLInputElement>('lib-search').addEventListener('input', (e) => {
    handleLibrarySearch((e.target as HTMLInputElement).value);
  });
  el('lib-search-clear').addEventListener('click', () => {
    (el('lib-search') as HTMLInputElement).value = '';
    handleLibrarySearch('');
    (el('lib-search') as HTMLInputElement).focus();
  });
})();

function parseTitle(title: string): PageContext {
  const atMatch = title.match(/^(.+?)\s+at\s+(.+?)(?:\s*[\|\-–].*)?$/i);
  if (atMatch) return { position: atMatch[1].trim(), company: atMatch[2].trim() };
  const dashMatch = title.match(/^(.+?)\s*[\-–\|]\s*(.+?)(?:\s*[\|\-–].*)?$/);
  if (dashMatch) return { company: dashMatch[1].trim(), position: dashMatch[2].trim() };
  return { company: '', position: title.trim() };
}

export {};

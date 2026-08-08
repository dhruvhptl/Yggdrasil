// Background service worker: manages the offline job and library queues.
// Flushes queued items every 5 minutes when the app is reachable.

const API_BASE = 'http://127.0.0.1:1421';
const API_KEY = 'ygg-local-dev';
const QUEUE_KEY = 'ygg_job_queue';
const LIBRARY_QUEUE_KEY = 'ygg_library_queue';

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

interface LibraryQueuedItem {
  url: string;
  title: string;
  note: string;
  queued_at: string;
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.action.setBadgeText({ text: '' });
  chrome.alarms.create('flush-queue', { periodInMinutes: 5 });
});

chrome.alarms.onAlarm.addListener(async (alarm) => {
  if (alarm.name !== 'flush-queue') return;

  try {
    const res = await fetch(`${API_BASE}/api/health`, {
      signal: AbortSignal.timeout(3000),
    });
    if (!res.ok) return;
  } catch {
    return;
  }

  await Promise.all([flushQueue(), flushLibraryQueue()]);
  await updateBadge();
});

async function flushQueue(): Promise<void> {
  const data = await chrome.storage.local.get(QUEUE_KEY);
  const queue: QueuedJob[] = data[QUEUE_KEY] ?? [];
  if (queue.length === 0) return;

  const remaining: QueuedJob[] = [];
  for (const job of queue) {
    try {
      const res = await fetch(`${API_BASE}/api/jobs`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'X-Ygg-Key': API_KEY,
        },
        body: JSON.stringify(job),
      });
      if (!res.ok) remaining.push(job);
    } catch {
      remaining.push(job);
    }
  }

  await chrome.storage.local.set({ [QUEUE_KEY]: remaining });
}

async function flushLibraryQueue(): Promise<void> {
  const data = await chrome.storage.local.get(LIBRARY_QUEUE_KEY);
  const queue: LibraryQueuedItem[] = (data[LIBRARY_QUEUE_KEY] as LibraryQueuedItem[]) ?? [];
  if (queue.length === 0) return;

  const remaining: LibraryQueuedItem[] = [];
  for (const item of queue) {
    try {
      const res = await fetch(`${API_BASE}/api/library`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'X-Ygg-Key': API_KEY },
        body: JSON.stringify(item),
      });
      if (!res.ok) remaining.push(item);
    } catch {
      remaining.push(item);
    }
  }
  await chrome.storage.local.set({ [LIBRARY_QUEUE_KEY]: remaining });
}

async function updateBadge(): Promise<void> {
  const [jobData, libData] = await Promise.all([
    chrome.storage.local.get(QUEUE_KEY),
    chrome.storage.local.get(LIBRARY_QUEUE_KEY),
  ]);
  const total =
    ((jobData[QUEUE_KEY] as unknown[]) ?? []).length +
    ((libData[LIBRARY_QUEUE_KEY] as unknown[]) ?? []).length;
  await chrome.action.setBadgeText({ text: total > 0 ? String(total) : '' });
  if (total > 0) await chrome.action.setBadgeBackgroundColor({ color: '#4f98a3' });
}

export {};

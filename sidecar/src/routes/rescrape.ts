import { Router, Request, Response } from 'express';
import fetch from 'node-fetch';
import { v4 as uuidv4 } from 'uuid';
import db from '../db.js';
import { getEmbedding } from '../embeddings.js';

export const rescrapeRouter = Router();

const SCRAPER_URL = 'http://127.0.0.1:3002';

// ── Shared chunking + embedding helpers ──────────────────────────────────────

function chunkText(text: string, maxWords = 400): string[] {
  const paragraphs = text.split(/\n\n+/).map((p) => p.trim()).filter(Boolean);
  const chunks: string[] = [];
  let current = '';

  for (const para of paragraphs) {
    const currentWords = current.split(/\s+/).filter(Boolean).length;
    const paraWords = para.split(/\s+/).filter(Boolean).length;

    if (currentWords + paraWords > maxWords) {
      if (current.trim()) chunks.push(current.trim());
      if (paraWords > maxWords) {
        const words = para.split(/\s+/);
        for (let i = 0; i < words.length; i += maxWords) {
          chunks.push(words.slice(i, i + maxWords).join(' '));
        }
        current = '';
      } else {
        current = para;
      }
    } else {
      current = current ? current + '\n\n' + para : para;
    }
  }
  if (current.trim()) chunks.push(current.trim());
  return chunks.filter((c) => c.length > 20);
}

/** Re-chunk + re-embed text for a resource. Returns new chunk count. */
async function storeChunksAndEmbeddings(resourceId: string, text: string): Promise<number> {
  const chunks = chunkText(text);
  console.log(`  → ${chunks.length} chunks to embed`);

  for (let i = 0; i < chunks.length; i++) {
    const chunk = chunks[i].replace(/[\x00-\x08\x0B\x0C\x0E-\x1F]/g, '');
    const chunkId = uuidv4();
    await db.query(
      'INSERT INTO mimir_chunks (id, resource_id, content, chunk_index) VALUES ($1, $2, $3, $4)',
      [chunkId, resourceId, chunk, i],
    );
    const embedding = await getEmbedding(chunk);
    const embeddingId = uuidv4();
    const vectorStr = `[${embedding.join(',')}]`;
    await db.query(
      'INSERT INTO mimir_embeddings (id, chunk_id, embedding) VALUES ($1, $2, $3::vector)',
      [embeddingId, chunkId, vectorStr],
    );
  }
  return chunks.length;
}

// ── Core rescrape logic ──────────────────────────────────────────────────────

interface RescrapeOneResult {
  success: boolean;
  changed: boolean;
  old_chunks: number;
  new_chunks: number;
  message: string;
  resource_id: string;
  title: string;
}

async function rescrapeOne(resourceId: string): Promise<RescrapeOneResult> {
  // Load resource
  const resourceResult = await db.query(
    'SELECT id, title, url, type FROM mimir_resources WHERE id = $1',
    [resourceId],
  );
  if (resourceResult.rows.length === 0) {
    return { success: false, changed: false, old_chunks: 0, new_chunks: 0, message: 'Resource not found', resource_id: resourceId, title: '' };
  }

  const resource = resourceResult.rows[0];

  if (resource.type !== 'webpage') {
    return { success: false, changed: false, old_chunks: 0, new_chunks: 0, message: 'Not a URL resource', resource_id: resourceId, title: resource.title };
  }
  if (!resource.url) {
    return { success: false, changed: false, old_chunks: 0, new_chunks: 0, message: 'No URL to fetch', resource_id: resourceId, title: resource.title };
  }

  // Count existing chunks and total content length
  const chunksResult = await db.query(
    'SELECT COUNT(*) AS count, COALESCE(SUM(LENGTH(content)), 0) AS total_length FROM mimir_chunks WHERE resource_id = $1',
    [resourceId],
  );
  const old_chunks = parseInt(chunksResult.rows[0].count, 10);
  const old_length = parseInt(chunksResult.rows[0].total_length, 10);

  // Fetch new content — NEVER delete before this succeeds
  let newText: string;
  try {
    const scraperRes = await fetch(`${SCRAPER_URL}/fetch`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: resource.url }),
    });

    if (scraperRes.ok) {
      const data = await scraperRes.json() as { text: string; char_count: number };
      newText = data.text;
    } else {
      const errBody = await scraperRes.json() as { detail?: string };
      throw new Error(errBody.detail || `Scraper returned ${scraperRes.status}`);
    }
  } catch (err: any) {
    // If fetch fails, return error WITHOUT touching existing chunks
    return {
      success: false, changed: false, old_chunks, new_chunks: 0,
      message: `Fetch failed: ${String(err)}`,
      resource_id: resourceId, title: resource.title,
    };
  }

  if (!newText || newText.length < 50) {
    return {
      success: false, changed: false, old_chunks, new_chunks: 0,
      message: 'Fetched text too short to use',
      resource_id: resourceId, title: resource.title,
    };
  }

  // If new content is not at least 20% longer, skip re-embedding
  if (newText.length < old_length * 1.2) {
    return {
      success: true, changed: false, old_chunks, new_chunks: old_chunks,
      message: 'Content similar to existing',
      resource_id: resourceId, title: resource.title,
    };
  }

  // Fetch succeeded and content is significantly richer — now delete old data
  await db.query(
    'DELETE FROM mimir_embeddings WHERE chunk_id IN (SELECT id FROM mimir_chunks WHERE resource_id = $1)',
    [resourceId],
  );
  await db.query('DELETE FROM mimir_chunks WHERE resource_id = $1', [resourceId]);

  // Re-chunk and re-embed with fresh content
  const new_chunks = await storeChunksAndEmbeddings(resourceId, newText);

  // Mark updated_at
  await db.query('UPDATE mimir_resources SET updated_at = NOW() WHERE id = $1', [resourceId]);

  console.log(`✅ Rescraped "${resource.title}": ${old_chunks} → ${new_chunks} chunks`);
  return {
    success: true, changed: true, old_chunks, new_chunks,
    message: `Updated: ${old_chunks} → ${new_chunks} chunks`,
    resource_id: resourceId, title: resource.title,
  };
}

// ── POST /rescrape/:resourceId ────────────────────────────────────────────────

rescrapeRouter.post('/:resourceId', async (req: Request, res: Response) => {
  const { resourceId } = req.params;
  try {
    const result = await rescrapeOne(resourceId);

    if (!result.success && result.message === 'Resource not found') {
      return res.status(404).json({ error: 'Resource not found' }) as unknown as void;
    }
    if (!result.success && result.message === 'Not a URL resource') {
      return res.status(400).json({ error: 'Only URL (webpage) resources can be re-scraped' }) as unknown as void;
    }
    if (!result.success) {
      return res.status(500).json({ error: result.message }) as unknown as void;
    }

    res.json({
      success: result.success,
      changed: result.changed,
      old_chunks: result.old_chunks,
      new_chunks: result.new_chunks,
      message: result.message,
    });
  } catch (err) {
    console.error('Rescrape error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── POST /rescrape (bulk, streams NDJSON) ─────────────────────────────────────

rescrapeRouter.post('/', async (req: Request, res: Response) => {
  res.setHeader('Content-Type', 'application/x-ndjson');
  res.setHeader('Cache-Control', 'no-cache');

  try {
    const resourcesResult = await db.query(
      "SELECT id, title FROM mimir_resources WHERE type = 'webpage' AND url IS NOT NULL ORDER BY created_at ASC",
    );
    const resources = resourcesResult.rows;
    const total = resources.length;

    let improved = 0, unchanged = 0, failed = 0;

    for (let i = 0; i < resources.length; i++) {
      if (i > 0) {
        await new Promise<void>((resolve) => setTimeout(resolve, 1000));
      }

      const resource = resources[i];
      const result = await rescrapeOne(resource.id);

      let status: 'improved' | 'unchanged' | 'failed';
      if (!result.success) {
        status = 'failed';
        failed++;
      } else if (result.changed) {
        status = 'improved';
        improved++;
      } else {
        status = 'unchanged';
        unchanged++;
      }

      console.log(`[rescrape] ${i + 1}/${total}: ${resource.title} — ${status}`);
      res.write(JSON.stringify({
        type: 'progress',
        current: i + 1,
        total,
        resourceId: resource.id,
        title: resource.title,
        status,
        oldChunks: result.old_chunks,
        newChunks: result.new_chunks,
      }) + '\n');
    }

    res.write(JSON.stringify({
      type: 'complete',
      improved,
      unchanged,
      failed,
    }) + '\n');

    res.end();
  } catch (err) {
    console.error('Rescrape-all error:', err);
    res.write(JSON.stringify({ type: 'error', message: String(err) }) + '\n');
    res.end();
  }
});

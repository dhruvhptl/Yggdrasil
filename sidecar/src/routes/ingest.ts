import { Router, Request, Response } from 'express';
import * as cheerio from 'cheerio';
import fetch from 'node-fetch';
import { v4 as uuidv4 } from 'uuid';
import { createHash } from 'crypto';
import pdfParse from 'pdf-parse';
import db from '../db.js';
import { getEmbedding } from '../embeddings.js';

export const ingestRouter = Router();

// ── Text chunking ─────────────────────────────────────────────────────────────

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

// ── Embed + store helper ─────────────────────────────────────────────────────

async function storeChunksAndEmbeddings(resourceId: string, text: string): Promise<void> {
  const chunks = chunkText(text);
  console.log(`  → ${chunks.length} chunks to embed`);

  for (let i = 0; i < chunks.length; i++) {
    // Strip null bytes + ASCII control chars Postgres rejects (keep \t \n \r)
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
}

// ── URL fetching ──────────────────────────────────────────────────────────────

const SCRAPER_URL = 'http://127.0.0.1:3002';

/** Fallback: plain node-fetch + cheerio (no JS rendering, no bot bypass) */
async function fetchWithNodeFetch(url: string): Promise<{ text: string; pageTitle: string }> {
  const fetchRes = await fetch(url, {
    headers: { 'User-Agent': 'Yggdrasil/1.0 (+https://github.com/yggdrasil-app)' },
  });
  if (!fetchRes.ok) {
    throw new Error(`Failed to fetch URL: ${fetchRes.status} ${fetchRes.statusText}`);
  }
  const html = await fetchRes.text();
  const $ = cheerio.load(html);
  $('script, style, nav, header, footer, aside, .nav, .menu, .sidebar, [class*="cookie"], [class*="banner"]').remove();
  const pageTitle = $('title').text().trim() || $('h1').first().text().trim() || url;
  const text = $('body').text().replace(/[ \t]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
  return { text, pageTitle };
}

interface ExternalLinkInfo {
  url: string;
  text: string;
  type: string;
}

interface FetchResult {
  text: string;
  fetcherUsed: string;
  pageType: string;
  externalLinks: ExternalLinkInfo[];
}

/** Primary: proxy through Scrapling service (handles Cloudflare, JS-rendered sites) */
async function fetchUrlContent(
  url: string,
  forceDynamic = false,
): Promise<FetchResult> {
  try {
    const scraperRes = await fetch(`${SCRAPER_URL}/fetch`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url, force_dynamic: forceDynamic }),
    });

    if (scraperRes.ok) {
      const data = await scraperRes.json() as {
        text: string;
        fetcher_used: string;
        char_count: number;
        page_type?: string;
        external_links?: { url: string; text: string; type: string }[];
      };
      console.log(`[scraper] ${data.fetcher_used} → ${data.char_count} chars, type=${data.page_type ?? 'article'}`);
      return {
        text: data.text,
        fetcherUsed: data.fetcher_used,
        pageType: data.page_type ?? 'article',
        externalLinks: data.external_links ?? [],
      };
    }

    const errBody = await scraperRes.json() as { detail?: string };
    throw new Error(errBody.detail || `Scraper returned ${scraperRes.status}`);
  } catch (err: any) {
    if (err.cause?.code === 'ECONNREFUSED' || err.message?.includes('ECONNREFUSED') || err.code === 'ECONNREFUSED') {
      console.warn('[scraper] Service unavailable — falling back to node-fetch');
      const { text } = await fetchWithNodeFetch(url);
      return { text, fetcherUsed: 'node-fetch (fallback)', pageType: 'article', externalLinks: [] };
    }
    throw err;
  }
}

// ── POST /ingest/url ─────────────────────────────────────────────────────────

ingestRouter.post('/url', async (req: Request, res: Response) => {
  const { url, title, force_dynamic, parent_id } = req.body as { url: string; title?: string; force_dynamic?: boolean; parent_id?: string };
  if (!url) return res.status(400).json({ error: 'url is required' }) as unknown as void;

  try {
    // Duplicate check — before fetching (fast path)
    const dupCheck = await db.query(
      "SELECT id, title, created_at FROM mimir_resources WHERE url = $1",
      [url],
    );
    if (dupCheck.rows.length > 0) {
      const existing = dupCheck.rows[0];
      return res.status(409).json({
        error: 'duplicate',
        message: 'This URL is already in your library',
        existing: { id: existing.id, title: existing.title, createdAt: existing.created_at },
      }) as unknown as void;
    }

    console.log(`📥 Ingesting URL: ${url}${force_dynamic ? ' (force dynamic)' : ''}`);

    const { text, fetcherUsed, pageType, externalLinks } = await fetchUrlContent(url, force_dynamic ?? false);

    if (text.length < 50) {
      return res
        .status(400)
        .json({ error: 'Could not extract meaningful text from URL' }) as unknown as void;
    }

    // Derive page title: use provided title, or fall back to a plain HEAD fetch for the <title> tag
    let pageTitle = title?.trim();
    if (!pageTitle) {
      try {
        const headRes = await fetch(url, { headers: { 'User-Agent': 'Yggdrasil/1.0' } });
        const html = await headRes.text();
        const $ = cheerio.load(html);
        pageTitle = $('title').text().trim() || $('h1').first().text().trim() || url;
      } catch {
        pageTitle = url;
      }
    }

    console.log(`  fetcher: ${fetcherUsed}, title: "${pageTitle}"`);

    const resourceId = uuidv4();
    if (parent_id) {
      await db.query(
        'INSERT INTO mimir_resources (id, title, url, type, status, parent_id) VALUES ($1, $2, $3, $4, $5, $6)',
        [resourceId, pageTitle, url, 'webpage', 'read', parent_id],
      );
    } else {
      await db.query(
        'INSERT INTO mimir_resources (id, title, url, type, status) VALUES ($1, $2, $3, $4, $5)',
        [resourceId, pageTitle, url, 'webpage', 'read'],
      );
    }

    await storeChunksAndEmbeddings(resourceId, text);

    console.log(`✅ Ingested URL: "${pageTitle}" (${resourceId}), type=${pageType}, ${externalLinks.length} external links`);
    res.json({ id: resourceId, title: pageTitle, pageType, externalLinks });
  } catch (err) {
    console.error('Ingest URL error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── POST /ingest/text ────────────────────────────────────────────────────────

ingestRouter.post('/text', async (req: Request, res: Response) => {
  const { text, title } = req.body as { text: string; title?: string };
  if (!text) return res.status(400).json({ error: 'text is required' }) as unknown as void;

  try {
    const resourceTitle = title?.trim() || 'Pasted text';

    // Duplicate check by content hash
    const contentHash = createHash('sha256').update(text.trim()).digest('hex');
    const dupCheck = await db.query(
      'SELECT id, title, created_at FROM mimir_resources WHERE content_hash = $1',
      [contentHash],
    );
    if (dupCheck.rows.length > 0) {
      const existing = dupCheck.rows[0];
      return res.status(409).json({
        error: 'duplicate',
        message: 'This text is already in your library',
        existing: { id: existing.id, title: existing.title, createdAt: existing.created_at },
      }) as unknown as void;
    }

    console.log(`📥 Ingesting text: "${resourceTitle}" (${text.length} chars)`);

    const resourceId = uuidv4();
    await db.query(
      'INSERT INTO mimir_resources (id, title, url, type, status, content_hash) VALUES ($1, $2, $3, $4, $5, $6)',
      [resourceId, resourceTitle, null, 'text', 'read', contentHash],
    );

    await storeChunksAndEmbeddings(resourceId, text);

    console.log(`✅ Ingested text: "${resourceTitle}" (${resourceId})`);
    res.json({ id: resourceId, title: resourceTitle });
  } catch (err) {
    console.error('Ingest text error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── POST /extract-pdf-text ───────────────────────────────────────────────────
// Returns full extracted text from a PDF without ingesting into Mimir.
// Used by the Resume page to get full text for Groq parsing.

ingestRouter.post('/extract-pdf-text', async (req: Request, res: Response) => {
  const { pdf_base64 } = req.body as { pdf_base64: string };
  if (!pdf_base64) return res.status(400).json({ error: 'pdf_base64 is required' }) as unknown as void;

  try {
    const buffer = Buffer.from(pdf_base64, 'base64');
    const pdfData = await pdfParse(buffer);
    const text = pdfData.text?.replace(/\0/g, '').trim();

    if (!text || text.length < 50) {
      return res.status(400).json({ error: 'Could not extract meaningful text from PDF' }) as unknown as void;
    }

    console.log(`📄 Extracted PDF text: ${text.length} chars, ${pdfData.numpages} pages`);
    res.json({ text, pages: pdfData.numpages, chars: text.length });
  } catch (err) {
    console.error('PDF text extraction error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── POST /ingest/pdf ─────────────────────────────────────────────────────────

ingestRouter.post('/pdf', async (req: Request, res: Response) => {
  const { filename, pdf_base64 } = req.body as { filename?: string; pdf_base64: string };
  if (!pdf_base64) return res.status(400).json({ error: 'pdf_base64 is required' }) as unknown as void;

  try {
    const buffer = Buffer.from(pdf_base64, 'base64');

    // Duplicate check by content hash of raw PDF bytes
    const contentHash = createHash('sha256').update(buffer).digest('hex');
    const dupCheck = await db.query(
      'SELECT id, title, created_at FROM mimir_resources WHERE content_hash = $1',
      [contentHash],
    );
    if (dupCheck.rows.length > 0) {
      const existing = dupCheck.rows[0];
      return res.status(409).json({
        error: 'duplicate',
        message: 'This PDF is already in your library',
        existing: { id: existing.id, title: existing.title, createdAt: existing.created_at },
      }) as unknown as void;
    }

    const pdfData = await pdfParse(buffer);
    // Strip null bytes — PostgreSQL UTF-8 rejects 0x00
    const text = pdfData.text?.replace(/\0/g, '').trim();

    if (!text || text.length < 50) {
      return res.status(400).json({ error: 'Could not extract meaningful text from PDF' }) as unknown as void;
    }

    const resourceTitle = filename?.replace(/\.pdf$/i, '').trim() || 'Uploaded PDF';
    console.log(`📥 Ingesting PDF: "${resourceTitle}" (${text.length} chars, ${pdfData.numpages} pages)`);

    const resourceId = uuidv4();
    await db.query(
      'INSERT INTO mimir_resources (id, title, url, type, status, content_hash) VALUES ($1, $2, $3, $4, $5, $6)',
      [resourceId, resourceTitle, null, 'pdf', 'read', contentHash],
    );

    await storeChunksAndEmbeddings(resourceId, text);

    console.log(`✅ Ingested PDF: "${resourceTitle}" (${resourceId})`);
    const textPreview = text.slice(0, 500);
    res.json({ id: resourceId, title: resourceTitle, textPreview });
  } catch (err) {
    console.error('Ingest PDF error:', err);
    res.status(500).json({ error: String(err) });
  }
});

import { Router, Request, Response } from 'express';
import * as cheerio from 'cheerio';
import fetch from 'node-fetch';
import { v4 as uuidv4 } from 'uuid';
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

// ── POST /ingest/url ─────────────────────────────────────────────────────────

ingestRouter.post('/url', async (req: Request, res: Response) => {
  const { url, title } = req.body as { url: string; title?: string };
  if (!url) return res.status(400).json({ error: 'url is required' }) as unknown as void;

  try {
    console.log(`📥 Ingesting URL: ${url}`);

    const fetchRes = await fetch(url, {
      headers: { 'User-Agent': 'Yggdrasil/1.0 (+https://github.com/yggdrasil-app)' },
    });

    if (!fetchRes.ok) {
      return res
        .status(400)
        .json({ error: `Failed to fetch URL: ${fetchRes.status} ${fetchRes.statusText}` }) as unknown as void;
    }

    const html = await fetchRes.text();
    const $ = cheerio.load(html);

    // Remove noise
    $('script, style, nav, header, footer, aside, .nav, .menu, .sidebar, [class*="cookie"], [class*="banner"]').remove();

    const pageTitle =
      title || $('title').text().trim() || $('h1').first().text().trim() || url;

    const rawText = $('body')
      .text()
      .replace(/[ \t]+/g, ' ')
      .replace(/\n{3,}/g, '\n\n')
      .trim();

    if (rawText.length < 50) {
      return res
        .status(400)
        .json({ error: 'Could not extract meaningful text from URL' }) as unknown as void;
    }

    const resourceId = uuidv4();
    await db.query(
      'INSERT INTO mimir_resources (id, title, url, type, status) VALUES ($1, $2, $3, $4, $5)',
      [resourceId, pageTitle, url, 'webpage', 'read'],
    );

    await storeChunksAndEmbeddings(resourceId, rawText);

    console.log(`✅ Ingested URL: "${pageTitle}" (${resourceId})`);
    res.json({ id: resourceId, title: pageTitle });
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
    console.log(`📥 Ingesting text: "${resourceTitle}" (${text.length} chars)`);

    const resourceId = uuidv4();
    await db.query(
      'INSERT INTO mimir_resources (id, title, url, type, status) VALUES ($1, $2, $3, $4, $5)',
      [resourceId, resourceTitle, null, 'text', 'read'],
    );

    await storeChunksAndEmbeddings(resourceId, text);

    console.log(`✅ Ingested text: "${resourceTitle}" (${resourceId})`);
    res.json({ id: resourceId, title: resourceTitle });
  } catch (err) {
    console.error('Ingest text error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── POST /ingest/pdf ─────────────────────────────────────────────────────────

ingestRouter.post('/pdf', async (req: Request, res: Response) => {
  const { filename, pdf_base64 } = req.body as { filename?: string; pdf_base64: string };
  if (!pdf_base64) return res.status(400).json({ error: 'pdf_base64 is required' }) as unknown as void;

  try {
    const buffer = Buffer.from(pdf_base64, 'base64');
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
      'INSERT INTO mimir_resources (id, title, url, type, status) VALUES ($1, $2, $3, $4, $5)',
      [resourceId, resourceTitle, null, 'pdf', 'read'],
    );

    await storeChunksAndEmbeddings(resourceId, text);

    console.log(`✅ Ingested PDF: "${resourceTitle}" (${resourceId})`);
    res.json({ id: resourceId, title: resourceTitle });
  } catch (err) {
    console.error('Ingest PDF error:', err);
    res.status(500).json({ error: String(err) });
  }
});

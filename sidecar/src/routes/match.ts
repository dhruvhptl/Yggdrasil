import { Router, Request, Response } from 'express';
import { v4 as uuidv4 } from 'uuid';
import db from '../db.js';
import { getEmbedding } from '../embeddings.js';

export const matchRouter = Router();

// ── POST /match/:nodeId ──────────────────────────────────────────────────────
// Embed the node's title+description, find similar mimir chunks,
// write to mimir_node_links, return matched resources.

matchRouter.post('/:nodeId', async (req: Request, res: Response) => {
  const { nodeId } = req.params;

  try {
    // 1. Fetch node title + description from DB
    const nodeResult = await db.query(
      'SELECT title, description FROM tree_nodes WHERE id = $1',
      [nodeId],
    );
    if (nodeResult.rows.length === 0) {
      return res.status(404).json({ error: 'Node not found' }) as unknown as void;
    }

    const { title, description } = nodeResult.rows[0] as { title: string; description: string };
    const searchText = [title, description].filter(Boolean).join(': ');

    // 2. Check if there are any embeddings to search against
    const countResult = await db.query('SELECT COUNT(*) AS n FROM mimir_embeddings');
    if (parseInt(countResult.rows[0].n as string, 10) === 0) {
      return res.json([]) as unknown as void;
    }

    // 3. Embed the node text and query pgvector
    const embedding = await getEmbedding(searchText);
    const vectorStr = `[${embedding.join(',')}]`;

    let similar: Array<{ resource_id: string; distance: number }>;
    try {
      const result = await db.query<{ resource_id: string; distance: number }>(
        `SELECT mc.resource_id,
                MIN(me.embedding <=> $1::vector) AS distance
         FROM mimir_embeddings me
         JOIN mimir_chunks mc ON mc.id = me.chunk_id
         GROUP BY mc.resource_id
         ORDER BY distance ASC
         LIMIT 5`,
        [vectorStr],
      );
      similar = result.rows;
    } catch (err: unknown) {
      const msg = (err as Error).message ?? '';
      if (msg.includes('operator does not exist') || msg.includes('type vector')) {
        console.error('pgvector not enabled — run: CREATE EXTENSION IF NOT EXISTS vector;');
        return res.status(503).json({ error: 'pgvector extension required' }) as unknown as void;
      }
      throw err;
    }

    // Only keep reasonably similar results (cosine distance < 0.8)
    const matches = similar.filter((r) => r.distance < 0.8);

    // 4. Upsert into mimir_node_links
    for (const match of matches) {
      const linkId = uuidv4();
      await db.query(
        `INSERT INTO mimir_node_links (id, resource_id, node_id, relevance_score)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (resource_id, node_id) DO UPDATE SET relevance_score = EXCLUDED.relevance_score`,
        [linkId, match.resource_id, nodeId, match.distance],
      );
    }

    if (matches.length === 0) return res.json([]) as unknown as void;

    // 5. Return the matched resources
    const resourceIds = matches.map((m) => m.resource_id);
    const resourceResult = await db.query(
      `SELECT id, title, url, type, status, created_at
       FROM mimir_resources
       WHERE id = ANY($1::text[])
       ORDER BY array_position($1::text[], id)`,
      [resourceIds],
    );

    console.log(`🔗 Matched ${matches.length} resources for node "${title}"`);
    res.json(resourceResult.rows);
  } catch (err) {
    console.error('Match error:', err);
    res.status(500).json({ error: String(err) });
  }
});

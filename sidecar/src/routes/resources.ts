import { Router, Request, Response } from 'express';
import db from '../db.js';

export const resourcesRouter = Router();

// ── GET /resources ───────────────────────────────────────────────────────────

resourcesRouter.get('/', async (_req: Request, res: Response) => {
  try {
    const result = await db.query(
      'SELECT id, title, url, type, status, user_notes, created_at FROM mimir_resources ORDER BY created_at DESC',
    );
    res.json(result.rows);
  } catch (err) {
    console.error('Get resources error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── GET /resources/by-node/:nodeId ───────────────────────────────────────────

resourcesRouter.get('/by-node/:nodeId', async (req: Request, res: Response) => {
  const { nodeId } = req.params;
  try {
    const result = await db.query(
      `SELECT mr.id, mr.title, mr.url, mr.type, mr.status, mr.user_notes, mr.created_at,
              mnl.relevance_score
       FROM mimir_resources mr
       JOIN mimir_node_links mnl ON mnl.resource_id = mr.id
       WHERE mnl.node_id = $1
       ORDER BY mnl.relevance_score ASC NULLS LAST`,
      [nodeId],
    );
    res.json(result.rows);
  } catch (err) {
    console.error('Get node resources error:', err);
    res.status(500).json({ error: String(err) });
  }
});

// ── DELETE /resources/:id ────────────────────────────────────────────────────

resourcesRouter.delete('/:id', async (req: Request, res: Response) => {
  const { id } = req.params;
  try {
    const result = await db.query('DELETE FROM mimir_resources WHERE id = $1', [id]);
    if ((result.rowCount ?? 0) === 0) {
      return res.status(404).json({ error: 'Resource not found' }) as unknown as void;
    }
    res.json({ ok: true });
  } catch (err) {
    console.error('Delete resource error:', err);
    res.status(500).json({ error: String(err) });
  }
});

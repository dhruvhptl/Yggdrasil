import { Router, Request, Response } from 'express';
import fetch from 'node-fetch';

export const playlistRouter = Router();

const SCRAPER_URL = 'http://127.0.0.1:3002';

playlistRouter.post('/', async (req: Request, res: Response) => {
  const { url } = req.body;
  if (!url) {
    return res.status(400).json({ error: 'url is required' }) as unknown as void;
  }

  try {
    const scraperRes = await fetch(`${SCRAPER_URL}/fetch-playlist`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url }),
      // @ts-ignore — node-fetch v2 timeout
      timeout: 30_000,
    });

    if (!scraperRes.ok) {
      const err = await scraperRes.json() as { detail?: string };
      return res.status(scraperRes.status).json({ error: err.detail || 'Playlist fetch failed' }) as unknown as void;
    }

    const data = await scraperRes.json();
    res.json(data);
  } catch (err: any) {
    const isDown = err.code === 'ECONNREFUSED' || String(err).includes('ECONNREFUSED');
    res.status(502).json({ error: isDown ? 'Scraper service not running' : String(err) });
  }
});

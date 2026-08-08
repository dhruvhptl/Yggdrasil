import express from 'express';
import cors from 'cors';
import { ingestRouter } from './routes/ingest.js';
import { resourcesRouter } from './routes/resources.js';
import { matchRouter } from './routes/match.js';
import { chatRouter } from './routes/chat.js';
import { rescrapeRouter } from './routes/rescrape.js';
import { discoverRouter } from './routes/discover.js';
import { playlistRouter } from './routes/playlist.js';

const app = express();
const PORT = process.env.MIMIR_PORT ? parseInt(process.env.MIMIR_PORT, 10) : 3001;

app.use(cors());
app.use(express.json({ limit: '200mb' }));
app.use(express.urlencoded({ limit: '200mb', extended: true }));

app.use('/ingest', ingestRouter);
app.use('/resources', resourcesRouter);
app.use('/match', matchRouter);
app.use('/chat', chatRouter);
app.use('/rescrape', rescrapeRouter);
app.use('/discover', discoverRouter);
app.use('/fetch-playlist', playlistRouter);

app.get('/health', (_req, res) => {
  res.json({ status: 'ok', name: 'Yggdrasil Mimir', port: PORT });
});

app.listen(PORT, () => {
  console.log(`🔮 Mimir sidecar running on http://localhost:${PORT}`);
});

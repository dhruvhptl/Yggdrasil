import { pipeline, env } from '@huggingface/transformers';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
// Cache models in sidecar/.model-cache to keep them out of the source tree
env.cacheDir = resolve(__dirname, '../../.model-cache');

// Lazy-initialize: reuse one promise so concurrent calls don't double-download
let pipelinePromise: ReturnType<typeof pipeline> | null = null;

export async function getEmbedding(text: string): Promise<number[]> {
  if (!pipelinePromise) {
    console.log('⏳ Loading all-MiniLM-L6-v2 embedding model (first run downloads ~90 MB)…');
    pipelinePromise = pipeline('feature-extraction', 'Xenova/all-MiniLM-L6-v2', {
      dtype: 'fp32',
    }).then((p) => {
      console.log('✅ Embedding model ready');
      return p;
    });
  }

  const extractor = await pipelinePromise;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const output: any = await extractor(text, { pooling: 'mean', normalize: true });
  return Array.from(output.data as Float32Array);
}

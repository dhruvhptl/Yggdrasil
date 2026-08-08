import { Router, Request, Response } from 'express';
import db from '../db.js';
import { getEmbedding } from '../embeddings.js';

export const chatRouter = Router();

interface ChatContext {
  page?: string;
  treeId?: string;
  nodeTitle?: string;
}

interface ChatRequest {
  message: string;
  context?: ChatContext;
}

chatRouter.post('/', async (req: Request, res: Response) => {
  const { message, context } = req.body as ChatRequest;
  if (!message) return res.status(400).json({ error: 'message is required' }) as unknown as void;

  try {
    console.log(`💬 Chat: "${message.slice(0, 80)}…"`);

    const apiKey = process.env.GROQ_API_KEY;
    if (!apiKey) {
      return res.status(500).json({ error: 'GROQ_API_KEY not configured' }) as unknown as void;
    }

    // 1. Embed the user's message (expand with node context if available)
    const queryText = context?.nodeTitle
      ? `${message} [context: ${context.nodeTitle}]`
      : message;
    const embedding = await getEmbedding(queryText);
    const vectorStr = `[${embedding.join(',')}]`;

    // 2. Check if any embeddings exist
    const countResult = await db.query('SELECT COUNT(*) AS n FROM mimir_embeddings');
    const embeddingCount = parseInt(countResult.rows[0].n as string, 10);

    let sources: { title: string; url: string | null; chunk: string; score: number }[] = [];
    let contextBlocks = '';

    if (embeddingCount > 0) {
      // 3. pgvector cosine similarity search — top 10 candidates
      //    <=> is cosine distance (0 = identical, 2 = opposite)
      //    threshold 0.85 to cast a wider net before reranking
      const result = await db.query<{
        content: string;
        resource_id: string;
        title: string;
        url: string | null;
        distance: number;
      }>(
        `SELECT mc.content, mc.resource_id, mr.title, mr.url,
                (me.embedding <=> $1::vector) AS distance
         FROM mimir_embeddings me
         JOIN mimir_chunks mc ON mc.id = me.chunk_id
         JOIN mimir_resources mr ON mr.id = mc.resource_id
         WHERE (me.embedding <=> $1::vector) < 0.85
         ORDER BY distance ASC
         LIMIT 10`,
        [vectorStr],
      );

      // 4. LLM reranking — pick the 3 most relevant chunks
      let rankedRows = result.rows;
      if (result.rows.length > 3) {
        try {
          const rerankChunks = result.rows
            .map((c, i) => `[${i}] ${c.content.slice(0, 200)}`)
            .join('\n');
          const rerankRes = await fetch('https://api.groq.com/openai/v1/chat/completions', {
            method: 'POST',
            headers: {
              Authorization: `Bearer ${apiKey}`,
              'Content-Type': 'application/json',
            },
            body: JSON.stringify({
              model: 'llama-3.1-8b-instant',
              messages: [
                {
                  role: 'system',
                  content:
                    'You are a relevance filter. Given a question and a list of numbered text chunks, respond with ONLY a JSON array of the indices (0-based) of the 3 most relevant chunks. Example: [0, 3, 7]',
                },
                {
                  role: 'user',
                  content: `Question: ${message}\n\nChunks:\n${rerankChunks}`,
                },
              ],
              temperature: 0,
              max_tokens: 200,
            }),
          });

          if (rerankRes.ok) {
            const rerankData = (await rerankRes.json()) as {
              choices?: { message?: { content?: string } }[];
            };
            const raw = rerankData.choices?.[0]?.message?.content ?? '';
            const match = raw.match(/\[[\d\s,]+\]/);
            if (match) {
              const indices: number[] = JSON.parse(match[0]);
              const picked = indices
                .filter((i) => i >= 0 && i < result.rows.length)
                .map((i) => result.rows[i]);
              if (picked.length > 0) {
                rankedRows = picked;
                console.log(`  ↳ reranker selected indices: ${indices.join(', ')}`);
              }
            }
          }
        } catch (rerankErr) {
          console.warn('Reranking failed, using top 3 by distance:', rerankErr);
          rankedRows = result.rows.slice(0, 3);
        }
      }

      sources = rankedRows.map((r) => ({
        title: r.title,
        url: r.url,
        chunk: r.content.slice(0, 300),
        score: parseFloat((1 - r.distance).toFixed(3)),
      }));

      contextBlocks = rankedRows
        .map((r, i) => `[${i + 1}] ${r.content}\n— Source: ${r.title}`)
        .join('\n\n');
    }

    // 5. If treeId present, fetch tree structure + linked resources
    let treeBlock = '';
    if (context?.treeId) {
      try {
        const treeResult = await db.query<{
          id: string;
          title: string;
          parent_id: string | null;
          resource_title: string | null;
          url: string | null;
          type: string | null;
        }>(
          `SELECT tn.id, tn.title, tn.parent_id,
                  mr.title as resource_title, mr.url, mr.type
           FROM tree_nodes tn
           LEFT JOIN mimir_node_links mnl ON mnl.node_id = tn.id
           LEFT JOIN mimir_resources mr ON mr.id = mnl.resource_id
           WHERE tn.tree_id = $1
           ORDER BY tn.order_index ASC`,
          [context.treeId],
        );

        if (treeResult.rows.length > 0) {
          // Group resources by node
          const nodeMap = new Map<string, { title: string; parentId: string | null; resources: string[] }>();
          for (const row of treeResult.rows) {
            if (!nodeMap.has(row.id)) {
              nodeMap.set(row.id, { title: row.title, parentId: row.parent_id, resources: [] });
            }
            if (row.resource_title) {
              const resLabel = `${row.resource_title}${row.type ? ` (${row.type})` : ''}${row.url ? ` — ${row.url}` : ''}`;
              nodeMap.get(row.id)!.resources.push(resLabel);
            }
          }

          const lines: string[] = [];
          for (const [id, node] of nodeMap) {
            const indent = node.parentId ? '  ' : '';
            lines.push(`${indent}- ${node.title} [id=${id}]`);
            for (const r of node.resources) {
              lines.push(`${indent}    ↳ ${r}`);
            }
          }
          treeBlock = `\n\nLearning Tree Structure:\n${lines.join('\n')}`;
          console.log(`  ↳ injected tree structure: ${nodeMap.size} nodes`);
        }
      } catch (treeErr) {
        console.warn('Failed to fetch tree structure:', treeErr);
      }
    }

    // 6. Build Groq system prompt
    const systemPrompt = `You are Mimir, a learning assistant embedded in Yggdrasil skill tree app. Answer based on the provided context chunks. If the context doesn't contain enough information, say so clearly and suggest what kind of resource would help. Always cite which chunk(s) you used.${
      context?.nodeTitle
        ? ` The user is currently on quest node: "${context.nodeTitle}". Bias your answer toward how it relates to that specific quest.`
        : ''
    }${
      treeBlock
        ? ` You have access to the user's learning tree and linked resources. When asked about sequencing, recommend order based on: prerequisite depth (shallower nodes first), resource type (short videos before dense docs), topological order of the tree.`
        : ''
    }${treeBlock}

Context:
${contextBlocks || '(No relevant resources found in your library.)'}`;

    // 7. Call Groq for synthesis
    const groqRes = await fetch('https://api.groq.com/openai/v1/chat/completions', {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${apiKey}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: 'llama-3.3-70b-versatile',
        messages: [
          { role: 'system', content: systemPrompt },
          { role: 'user', content: message },
        ],
        temperature: 0.4,
        max_tokens: 2048,
      }),
    });

    if (!groqRes.ok) {
      const errText = await groqRes.text();
      console.error('Groq error:', errText);
      return res.status(502).json({ error: `Groq API error: ${groqRes.status}` }) as unknown as void;
    }

    const groqData = (await groqRes.json()) as {
      choices?: { message?: { content?: string } }[];
    };
    const answer = groqData.choices?.[0]?.message?.content || 'No response from AI.';

    console.log(`✅ Chat response: ${answer.length} chars, ${sources.length} sources`);
    res.json({ answer, sources });
  } catch (err) {
    console.error('Chat error:', err);
    res.status(500).json({ error: String(err) });
  }
});

# Yggdrasil

A personal learning OS. Paste a project repo or PRD, get an AI-generated skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

## Architecture

```
React frontend (Vite, port 1420)
       ↓
Rust backend (Tauri commands)  ←→  Neon Postgres (pgvector, vector(1024))
                                ←→  OpenRouter (tree gen: Kimi K2 + Gemini Flash)
                                ←→  Groq (chat synthesis + reranking: LLaMA 3.3-70b)

Python scraper (port 3002)     ←→  OpenRouter (embeddings: pplx-embed-v1-0.6b)
```

Mimir (resource ingestion, RAG, embeddings) runs as **native Rust** inside the Tauri process — no separate sidecar.

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Backend | Rust + sqlx |
| Database | Postgres on Neon (pgvector, `vector(1024)`) |
| Tree generation | Kimi K2 via OpenRouter (outline + checkpoint expansion) |
| Concept graph | Gemini Flash via OpenRouter |
| Chat + extraction | Groq — LLaMA 3.3-70b-versatile + 3.1-8b-instant (reranker) |
| Embeddings | Perplexity pplx-embed-v1-0.6b (1024-dim) via OpenRouter |
| Tree renderer | Custom HTML Canvas (L-system procedural branches) |
| Visualizations | D3 force simulation |
| Mimir | Native Rust in `mimir.rs` — no sidecar |
| Scraper | Python FastAPI (port 3002) — URL scraping, PDF extraction (pymupdf), playlists |

## Prerequisites

| Tool | Install | Version |
|------|---------|---------|
| **Git** | [git-scm.com](https://git-scm.com/) | Any recent |
| **Node.js** | [nodejs.org](https://nodejs.org/) | 18+ |
| **Rust + Cargo** | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` | stable |
| **Python** | [python.org](https://www.python.org/) | 3.10+ |
| **Bitwarden CLI** | `npm install -g @bitwarden/cli` | Any |

### Windows-specific (Tauri prerequisites)

- **WebView2** — pre-installed on Windows 10/11. If missing: [download from Microsoft](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- **Visual Studio Build Tools** — install via [Visual Studio Installer](https://visualstudio.microsoft.com/visual-cpp-build-tools/). Select "Desktop development with C++".

### macOS-specific

```bash
xcode-select --install
```

## Setup

```bash
# Clone
git clone https://github.com/your-username/Yggdrasil.git
cd Yggdrasil

# Install frontend dependencies
npm install

# Install Python scraper dependencies
cd scraper
python -m venv venv
source venv/bin/activate  # Windows: venv\Scripts\activate
pip install -r requirements.txt
cd ..
```

## Environment Variables

All secrets are stored in Bitwarden and exported into the process by `dev.sh` — no `.env` file is read at runtime.

Store the following as **password** items in your Bitwarden vault:

| Bitwarden Item Name | What It Is |
|---------------------|------------|
| `DATABASE_URL` | Postgres connection string (e.g. `postgresql://user:pass@host.neon.tech/neondb?sslmode=require`) |
| `GROQ_API_KEY` | [Groq API key](https://console.groq.com/) — Mimir chat + skill extraction |
| `OPENROUTER_API_KEY` | [OpenRouter API key](https://openrouter.ai/) — tree generation + embeddings |
| `YOUTUBE_API_KEY` | [YouTube Data API key](https://console.cloud.google.com/) — playlist ingestion |
| `GITHUB_TOKEN` | [GitHub personal access token](https://github.com/settings/tokens) — repo analysis |

`dev.sh` derives the following from the above automatically:

```
TREE_GEN_API_KEY        = OPENROUTER_API_KEY
TREE_GEN_BASE_URL       = https://openrouter.ai/api/v1/chat/completions
TREE_GEN_MODEL          = moonshotai/kimi-k2
CONCEPT_GRAPH_MODEL     = google/gemini-2.5-flash
CONCEPT_GRAPH_BASE_URL  = https://openrouter.ai/api/v1/chat/completions
CONCEPT_GRAPH_API_KEY   = OPENROUTER_API_KEY
```

## Run

```bash
bash dev.sh
```

This will:
1. Prompt for your Bitwarden master password
2. Export all environment variables into the process
3. Start two services concurrently:
   - **Vite** dev server (port 1420)
   - **Python scraper** (port 3002)
4. Build and launch the Tauri desktop app
5. Run all database migrations automatically on startup

## Features

- **Skill Trees** — AI generates learning trees from project PRDs or GitHub repos (two-stage: concept graph → outline → per-skill checkpoint expansion)
- **Checkpoints** — Track progress with mastery criteria, exercises, and notes
- **Daily Matrix** — Eisenhower 2x2 triage for daily learning priorities
- **Resource Library** — Ingest URLs, PDFs, YouTube playlists; auto-tag; track completion; auto-match to tree nodes via pgvector
- **Mimir Chat** — RAG assistant grounded in your personal resource library, with section + page citations
- **Work Tracker** — Co-op experience with D3 galaxy visualization and AI skill extraction
- **Job Tracker** — Kanban board with JD analysis and skill demand analytics
- **Ideas** — Scratchpad with tagging, pinning, and promote-to-project
- **Resume** — AI-parsed resume that seeds your skill inventory
- **Universal Skill Tree** — D3 force galaxy aggregating skills from all sources with gap analysis

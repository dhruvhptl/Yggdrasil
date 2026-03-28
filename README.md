# Yggdrasil

A personal learning OS. Paste a project repo or PRD, get an AI-generated skill tree of everything behind what you built. Co-op experience, job applications, and learning progress all feed into a Universal Skill Tree — your living proof of expertise.

## Prerequisites

Install the following on a fresh machine:

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

# Install frontend + sidecar dependencies
npm install
npm --prefix sidecar install

# Install Python scraper dependencies
cd scraper
python -m venv venv
source venv/bin/activate  # Windows: venv\Scripts\activate
pip install -r requirements.txt
cd ..
```

## Environment Variables

Yggdrasil uses Bitwarden CLI to inject secrets at runtime. The `dev.sh` script handles this automatically.

### Required Bitwarden entries

Store the following as **password** items in your Bitwarden vault:

| Bitwarden Item Name | What It Is |
|---------------------|------------|
| `DATABASE_URL` | Postgres connection string (e.g. Neon: `postgresql://user:pass@host.neon.tech/neondb?sslmode=require`) |
| `GROQ_API_KEY` | [Groq API key](https://console.groq.com/) — used for Mimir chat + skill extraction |
| `OPENROUTER_API_KEY` | [OpenRouter API key](https://openrouter.ai/) — used for tree generation (Kimi K2 + Gemini Flash) |
| `YOUTUBE_API_KEY` | [YouTube Data API key](https://console.cloud.google.com/) — for playlist ingestion |
| `GITHUB_TOKEN` | [GitHub personal access token](https://github.com/settings/tokens) — for repo analysis |

### Alternative: manual `.env` file

If you prefer not to use Bitwarden, create `src-tauri/.env`:

```bash
DATABASE_URL=postgresql://user:pass@host.neon.tech/neondb?sslmode=require
GROQ_API_KEY=gsk_...
OPENROUTER_API_KEY=sk-or-...
TREE_GEN_API_KEY=sk-or-...  # same as OPENROUTER_API_KEY
TREE_GEN_BASE_URL=https://openrouter.ai/api/v1/chat/completions
TREE_GEN_MODEL=moonshotai/kimi-k2
CONCEPT_GRAPH_MODEL=google/gemini-2.5-flash
CONCEPT_GRAPH_BASE_URL=https://openrouter.ai/api/v1/chat/completions
CONCEPT_GRAPH_API_KEY=sk-or-...  # same as OPENROUTER_API_KEY
YOUTUBE_API_KEY=...
GITHUB_TOKEN=ghp_...
```

Then run `npm run tauri dev` directly instead of `bash dev.sh`.

## Run

```bash
bash dev.sh
```

This will:
1. Prompt for your Bitwarden master password
2. Export all environment variables
3. Start three services concurrently:
   - **Vite** dev server (port 1420)
   - **Mimir** sidecar (port 3001)
   - **Scraper** service (port 3002)
4. Build and launch the Tauri desktop app
5. Run all database migrations automatically on startup

## Tech Stack

| Layer | Technology |
|---|---|
| Frontend | React 19 + TypeScript + Vite + Tailwind |
| Desktop | Tauri 2.0 |
| Backend | Rust + sqlx |
| Database | Postgres on Neon (pgvector) |
| AI | OpenRouter (Kimi K2, Gemini Flash) + Groq (LLaMA 3.3-70b) |
| Tree renderer | Custom HTML Canvas (L-system) |
| Visualizations | D3 force simulation |
| Mimir sidecar | Node.js + TypeScript |
| Embeddings | Transformers.js (all-MiniLM-L6-v2, 384d) |
| Scraper | Python FastAPI |

## Features

- **Skill Trees** — AI generates learning trees from project PRDs or GitHub repos
- **Checkpoints** — Track progress through quests with mastery criteria and exercises
- **Daily Matrix** — Eisenhower 2x2 triage for daily learning priorities
- **Resource Library** — Ingest URLs, PDFs, text; auto-tag; track completion; match to tree nodes
- **Mimir Chat** — RAG assistant grounded in your personal resource library
- **Work Tracker** — Co-op experience with D3 galaxy visualization and AI skill extraction
- **Job Tracker** — Kanban board with JD analysis and skill demand analytics
- **Ideas** — Scratchpad with tagging, pinning, and promote-to-project
- **Resume** — AI-parsed resume that seeds your skill inventory
- **Universal Skill Tree** — D3 force galaxy aggregating skills from all sources with gap analysis

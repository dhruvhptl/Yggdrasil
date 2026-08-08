#!/bin/bash

# Yggdrasil dev.sh - Bitwarden env loader + launch

set -e

# Clean up the Hound sidecar when this script exits (Ctrl+C, app quit, or error)
trap 'taskkill //IM hound.exe //F >/dev/null 2>&1 || true' EXIT

# Ensure BW_SESSION is set for vault access
if [ -z "$BW_SESSION" ]; then
    echo "BW_SESSION not set. Unlocking Bitwarden..."
    echo -n "Bitwarden master password: "
    read -s PASSWORD
    echo
    TMPFILE=$(mktemp)
    echo "$PASSWORD" > "$TMPFILE"
    export BW_SESSION=$(bw unlock --passwordfile "$TMPFILE" --raw 2>/dev/null || echo "")
    rm -f "$TMPFILE"
    if [ -z "$BW_SESSION" ]; then
        echo "Failed to unlock Bitwarden."
        exit 1
    fi
fi

# Retrieve secrets from already-unlocked Bitwarden vault
export DATABASE_URL=$(bw get password "DATABASE_URL" 2>/dev/null || echo "")
export GROQ_API_KEY=$(bw get password "GROQ_API_KEY" 2>/dev/null || echo "")
export GITHUB_TOKEN=$(bw get password "GITHUB_TOKEN" 2>/dev/null || echo "")
export OPENROUTER_API_KEY=$(bw get password "OPENROUTER_API_KEY" 2>/dev/null || echo "")
export YOUTUBE_API_KEY=$(bw get password "YOUTUBE_API_KEY" 2>/dev/null || echo "")

# Derived env vars
export TREE_GEN_BASE_URL="https://openrouter.ai/api/v1/chat/completions"
export TREE_GEN_API_KEY="$OPENROUTER_API_KEY"
export TREE_GEN_MODEL="google/gemini-2.5-flash"
export CONCEPT_GRAPH_MODEL="google/gemini-2.5-flash"
export CONCEPT_GRAPH_BASE_URL="https://openrouter.ai/api/v1/chat/completions"
export CONCEPT_GRAPH_API_KEY="$OPENROUTER_API_KEY"
# Agent chat model — DeepSeek V4 Flash via OpenRouter (1M ctx, tool_choice support).
# Fallback: swap MODEL to google/gemini-2.5-flash or qwen/qwen3-coder-flash — no code change.
# (Qwen-coder-vs-DeepSeek per-turn routing lands with the project/FS tool schema.)
export MIMIR_AGENT_MODEL="deepseek/deepseek-v4-flash"
export MIMIR_AGENT_BASE_URL="https://openrouter.ai/api/v1/chat/completions"
export MIMIR_AGENT_API_KEY="$OPENROUTER_API_KEY"
# export MIMIR_AGENT_ENABLED="false"   # uncomment to force the classic one-shot chat path
export MIMIR_PORT="3001"

# Optional Mimir retrieval tuning (defaults shown — unset to use defaults)
# export MIMIR_TOP_K="10"          # pgvector candidates fetched per query
# export MIMIR_THRESHOLD="0.85"    # cosine distance cutoff (lower = stricter)
# export MIMIR_RERANK_TOP_N="3"    # chunks kept after LLM rerank
# export MIMIR_PREMATCH_BOOST="true" # inject node's pre-matched chunks into context
# export MIMIR_LEXICAL_TOP_K="10"  # BM25 lexical candidates fetched per query
# export MIMIR_HYBRID_WEIGHT="0.5" # reserved for weighted combination (RRF is default)

echo "✓ Environment loaded from Bitwarden"

# Start Hound web-research sidecar (optional; MCP server on 127.0.0.1:8765)
HOUND_EXE="$APPDATA/Python/Python313/Scripts/hound.exe"
if [ -f "$HOUND_EXE" ]; then
    echo "🚀 Starting Hound on port 8765..."
    taskkill //IM hound.exe //F >/dev/null 2>&1 || true
    "$HOUND_EXE" --http --port 8765 >"${TEMP:-/tmp}/hound.log" 2>&1 &
    sleep 2
    echo "✓ Hound launched in background (logs: ${TEMP:-/tmp}/hound.log)"
else
    echo "⚠ hound.exe not found at $HOUND_EXE — web tools disabled"
fi

# Write env file to Tauri app data dir (keeps it current for production builds too)
APP_DATA_DIR="$APPDATA/com.universal.skilltree"
mkdir -p "$APP_DATA_DIR"
cat > "$APP_DATA_DIR/.env" <<EOF
DATABASE_URL=$DATABASE_URL
GROQ_API_KEY=$GROQ_API_KEY
GITHUB_TOKEN=$GITHUB_TOKEN
OPENROUTER_API_KEY=$OPENROUTER_API_KEY
YOUTUBE_API_KEY=$YOUTUBE_API_KEY
TREE_GEN_BASE_URL=$TREE_GEN_BASE_URL
TREE_GEN_API_KEY=$TREE_GEN_API_KEY
TREE_GEN_MODEL=$TREE_GEN_MODEL
CONCEPT_GRAPH_MODEL=$CONCEPT_GRAPH_MODEL
CONCEPT_GRAPH_BASE_URL=$CONCEPT_GRAPH_BASE_URL
CONCEPT_GRAPH_API_KEY=$CONCEPT_GRAPH_API_KEY
MIMIR_AGENT_MODEL=$MIMIR_AGENT_MODEL
MIMIR_AGENT_BASE_URL=$MIMIR_AGENT_BASE_URL
MIMIR_AGENT_API_KEY=$MIMIR_AGENT_API_KEY
EOF

# Launch the app
npm run tauri dev

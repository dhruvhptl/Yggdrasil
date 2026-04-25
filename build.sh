#!/bin/bash

# Yggdrasil build.sh - Bitwarden env loader + production build

set -e

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
export MIMIR_PORT="3001"

echo "✓ Environment loaded from Bitwarden"

# Write env file to Tauri app data dir so the installed app can read it at runtime
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
EOF
echo "✓ .env written to $APP_DATA_DIR"

# Build the app
npm run tauri build

// src/components/MimirChat.tsx
import { useState, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocation } from "react-router-dom";
import { Send, X, Loader2, ExternalLink } from "lucide-react";
import { useMimirContext } from "../contexts/MimirContext";

interface Source {
  title: string;
  url: string | null;
  chunk: string;
  score: number;
}

interface ChatMessage {
  role: "user" | "mimir";
  content: string;
  sources?: Source[];
}

interface MimirChatResponse {
  answer: string;
  sources: Source[];
}

export default function MimirChat({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  const location = useLocation();
  const { treeId, nodeTitle } = useMimirContext();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Focus textarea when panel opens
  useEffect(() => {
    if (open) {
      setTimeout(() => textareaRef.current?.focus(), 200);
    }
  }, [open]);

  // Derive current page context from route
  const page = location.pathname === "/" ? "home" : location.pathname.slice(1);

  async function handleSend() {
    const msg = input.trim();
    if (!msg || loading) return;

    setInput("");
    setMessages((prev) => [...prev, { role: "user", content: msg }]);
    setLoading(true);

    try {
      const response = await invoke<MimirChatResponse>("mimir_chat", {
        message: msg,
        page,
        treeId,
        nodeTitle,
      });

      setMessages((prev) => [
        ...prev,
        {
          role: "mimir",
          content: response.answer,
          sources: response.sources,
        },
      ]);
    } catch (e) {
      setMessages((prev) => [
        ...prev,
        {
          role: "mimir",
          content: `Error: ${e}`,
        },
      ]);
    } finally {
      setLoading(false);
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }

  return (
    <div
      style={{
        width: open ? 380 : 0,
        flexShrink: 0,
        overflow: "hidden",
        transition: "width 0.2s ease",
        borderLeft: open ? "1px solid #1e293b" : "none",
      }}
    >
      <div
        style={{
          width: 380,
          height: "100%",
          display: "flex",
          flexDirection: "column",
          background: "#0f172a",
        }}
      >
        {/* Header */}
        <div
          style={{
            padding: "12px 16px",
            borderBottom: "1px solid #1e293b",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            flexShrink: 0,
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{ fontSize: 16 }}>🔮</span>
            <span
              style={{
                fontWeight: 600,
                fontSize: 14,
                color: "#f1f5f9",
                textShadow: "0 0 12px rgba(16, 185, 129, 0.3)",
              }}
            >
              Mimir
            </span>
            <span style={{ fontSize: 10, color: "#475569" }}>
              RAG assistant
            </span>
          </div>
          <button
            onClick={onToggle}
            style={{
              background: "none",
              border: "none",
              color: "#64748b",
              cursor: "pointer",
              padding: 4,
            }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Messages */}
        <div
          style={{
            flex: 1,
            overflowY: "auto",
            padding: "12px 14px",
            display: "flex",
            flexDirection: "column",
            gap: 12,
          }}
        >
          {messages.length === 0 && (
            <div
              style={{
                textAlign: "center",
                color: "#475569",
                fontSize: 12,
                marginTop: 40,
                lineHeight: 1.6,
              }}
            >
              Ask Mimir about your resources.
              <br />
              Answers are grounded in your personal library.
            </div>
          )}

          {messages.map((msg, i) => (
            <div key={i}>
              {msg.role === "user" ? (
                <div style={{ display: "flex", justifyContent: "flex-end" }}>
                  <div
                    style={{
                      maxWidth: "85%",
                      padding: "8px 12px",
                      borderRadius: "12px 12px 2px 12px",
                      background: "rgba(16, 185, 129, 0.12)",
                      border: "1px solid rgba(16, 185, 129, 0.25)",
                      color: "#d1fae5",
                      fontSize: 13,
                      lineHeight: 1.5,
                      whiteSpace: "pre-wrap",
                    }}
                  >
                    {msg.content}
                  </div>
                </div>
              ) : (
                <div>
                  <div
                    style={{
                      maxWidth: "90%",
                      padding: "10px 12px",
                      borderRadius: "12px 12px 12px 2px",
                      background: "rgba(30, 41, 59, 0.6)",
                      border: "1px solid #334155",
                      color: "#e2e8f0",
                      fontSize: 13,
                      lineHeight: 1.6,
                      whiteSpace: "pre-wrap",
                    }}
                  >
                    {msg.content}
                  </div>
                  {/* Source badges */}
                  {msg.sources && msg.sources.length > 0 && (
                    <div
                      style={{
                        display: "flex",
                        flexWrap: "wrap",
                        gap: 4,
                        marginTop: 6,
                      }}
                    >
                      {msg.sources.map((src, j) => (
                        <SourceBadge key={j} source={src} />
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}

          {loading && (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                color: "#64748b",
                fontSize: 12,
              }}
            >
              <Loader2
                size={14}
                style={{ animation: "spin 1s linear infinite" }}
              />
              Searching library…
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>

        {/* Input */}
        <div
          style={{
            padding: "10px 14px",
            borderTop: "1px solid #1e293b",
            flexShrink: 0,
          }}
        >
          <div
            style={{
              display: "flex",
              gap: 8,
              alignItems: "flex-end",
            }}
          >
            <textarea
              ref={textareaRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Ask about your resources…"
              rows={1}
              style={{
                flex: 1,
                padding: "8px 10px",
                background: "#1e293b",
                border: "1px solid #334155",
                borderRadius: 8,
                color: "#f1f5f9",
                fontSize: 13,
                resize: "none",
                fontFamily: "inherit",
                lineHeight: 1.4,
                maxHeight: 80,
                outline: "none",
              }}
              onInput={(e) => {
                const target = e.target as HTMLTextAreaElement;
                target.style.height = "auto";
                target.style.height =
                  Math.min(target.scrollHeight, 80) + "px";
              }}
            />
            <button
              onClick={handleSend}
              disabled={!input.trim() || loading}
              style={{
                padding: "8px 10px",
                background: input.trim() && !loading ? "#059669" : "#1e293b",
                border: "none",
                borderRadius: 8,
                color:
                  input.trim() && !loading ? "#ffffff" : "#475569",
                cursor:
                  input.trim() && !loading ? "pointer" : "default",
                flexShrink: 0,
                transition: "background 0.15s",
              }}
            >
              <Send size={14} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Source Badge ────────────────────────────────────────────────────────────

function SourceBadge({ source }: { source: Source }) {
  const badge = (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 3,
        padding: "2px 8px",
        borderRadius: 6,
        background: "#1e293b",
        border: "1px solid #334155",
        color: "#94a3b8",
        fontSize: 10,
        cursor: source.url ? "pointer" : "default",
        maxWidth: 200,
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
      }}
      title={`${source.title} (${Math.round(source.score * 100)}% match)\n${source.chunk}`}
    >
      {source.title}
      <span style={{ color: "#475569" }}>
        {Math.round(source.score * 100)}%
      </span>
      {source.url && <ExternalLink size={9} />}
    </span>
  );

  if (source.url) {
    return (
      <a
        href={source.url}
        target="_blank"
        rel="noopener noreferrer"
        style={{ textDecoration: "none" }}
      >
        {badge}
      </a>
    );
  }

  return badge;
}

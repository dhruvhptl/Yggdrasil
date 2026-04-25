// src/components/MimirChat.tsx
import { useState, useRef, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocation } from "react-router-dom";
import { Send, X, Loader2, ExternalLink, Trash2, BarChart2, Cpu } from "lucide-react";
import { useMimirContext } from "../contexts/MimirContext";
import { validateOrLog, MimirChatResponseSchema } from "../lib/validators";
import type { NodeChatContext } from "../types";

interface Source {
  title: string;
  url: string | null;
  chunk: string;
  score: number;
  sectionTitle: string | null;
  pageStart: number | null;
  pageEnd: number | null;
}

interface ChatMessage {
  role: "user" | "mimir";
  content: string;
  sources?: Source[];
  createdAt?: string;
}

interface StoredChatMessage {
  id: string;
  role: string;
  content: string;
  sources: Source[] | null;
  createdAt: string;
}

interface MimirChatResponse {
  answer: string;
  sources: Source[];
}

interface RetrievalStats {
  totalQueries: number;
  avgCandidatesBeforeRerank: number;
  avgCandidatesAfterRerank: number;
  rerankFallbackRate: number;
  avgPrematchChunksUsed: number;
  topQueriedNodes: { nodeId: string; queryCount: number }[];
}

interface CommandStats {
  command: string;
  totalCalls: number;
  successRate: number;
  avgLatencyMs: number;
  latestPromptVersion: string;
}

interface ModelStats {
  model: string;
  totalCalls: number;
  avgLatencyMs: number;
  errorRate: number;
}

interface RecentError {
  command: string;
  error: string;
  createdAt: string;
}

interface PromptStats {
  byCommand: CommandStats[];
  byModel: ModelStats[];
  recentErrors: RecentError[];
}

export default function MimirChat({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: () => void;
}) {
  const location = useLocation();
  const { treeId, nodeId, nodeTitle } = useMimirContext();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [resumed, setResumed] = useState(false);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [showStats, setShowStats] = useState(false);
  const [stats, setStats] = useState<RetrievalStats | null>(null);
  const [showPromptStats, setShowPromptStats] = useState(false);
  const [promptStats, setPromptStats] = useState<PromptStats | null>(null);
  const [nodeContext, setNodeContext] = useState<NodeChatContext | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const loadedSessionKey = useRef<string | null>(null);

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

  // Load session history when panel opens or context changes
  const sessionKey = treeId && nodeId ? `${treeId}:${nodeId}` : null;
  const loadSession = useCallback(async () => {
    if (!treeId || !nodeId) {
      setMessages([]);
      setResumed(false);
      setNodeContext(null);
      loadedSessionKey.current = null;
      return;
    }
    if (loadedSessionKey.current === sessionKey) return;
    loadedSessionKey.current = sessionKey;

    // Fetch chat history and node context in parallel
    const [storedResult, ctxResult] = await Promise.allSettled([
      invoke<StoredChatMessage[]>("get_chat_session", { treeId, nodeId }),
      invoke<NodeChatContext>("get_node_chat_context", { nodeId }),
    ]);

    if (ctxResult.status === "fulfilled") {
      setNodeContext(ctxResult.value);
    }

    if (storedResult.status === "fulfilled" && storedResult.value.length > 0) {
      const loaded: ChatMessage[] = storedResult.value.map((m) => ({
        role: m.role === "user" ? "user" : "mimir",
        content: m.content,
        sources: m.sources ?? undefined,
        createdAt: m.createdAt,
      }));
      setMessages(loaded);
      setResumed(true);
    } else {
      setMessages([]);
      setResumed(false);
    }
  }, [treeId, nodeId, sessionKey]);

  useEffect(() => {
    if (open) loadSession();
  }, [open, loadSession]);

  async function handleShowStats() {
    if (showStats) { setShowStats(false); return; }
    try {
      const s = await invoke<RetrievalStats>("get_retrieval_stats");
      setStats(s);
      setShowStats(true);
    } catch { /* best-effort */ }
  }

  async function handleShowPromptStats() {
    if (showPromptStats) { setShowPromptStats(false); return; }
    try {
      const s = await invoke<PromptStats>("get_prompt_stats");
      setPromptStats(s);
      setShowPromptStats(true);
    } catch { /* best-effort */ }
  }

  async function handleClear() {
    if (!treeId || !nodeId) return;
    try {
      await invoke("clear_chat_session", { treeId, nodeId });
    } catch { /* best-effort */ }
    setMessages([]);
    setResumed(false);
    loadedSessionKey.current = null;
    // Keep nodeContext — resources are still relevant after clearing chat
  }

  // Derive current page context from route
  const page = location.pathname === "/" ? "home" : location.pathname.slice(1);

  async function handleSend() {
    const msg = input.trim();
    if (!msg || loading) return;

    setInput("");
    setMessages((prev) => [...prev, { role: "user", content: msg }]);
    setLoading(true);

    try {
      const raw = await invoke("mimir_chat", {
        message: msg,
        page,
        treeId,
        nodeId,
        nodeTitle,
      });
      const response = validateOrLog(MimirChatResponseSchema, raw, 'mimir_chat') as MimirChatResponse;
      setResumed(false);

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
          position: "relative",
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
          <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
            {import.meta.env.DEV && (
              <>
                <button
                  onClick={handleShowPromptStats}
                  title="Model logs"
                  style={{
                    background: showPromptStats ? "rgba(99,102,241,0.15)" : "none",
                    border: "none",
                    color: showPromptStats ? "#818cf8" : "#475569",
                    cursor: "pointer",
                    padding: 4,
                    display: "flex",
                    alignItems: "center",
                    borderRadius: 4,
                  }}
                >
                  <Cpu size={13} />
                </button>
                <button
                  onClick={handleShowStats}
                  title="Retrieval stats"
                  style={{
                    background: showStats ? "rgba(16,185,129,0.15)" : "none",
                    border: "none",
                    color: showStats ? "#10b981" : "#475569",
                    cursor: "pointer",
                    padding: 4,
                    display: "flex",
                    alignItems: "center",
                    borderRadius: 4,
                  }}
                >
                  <BarChart2 size={13} />
                </button>
              </>
            )}
            {treeId && nodeId && messages.length > 0 && (
              <button
                onClick={handleClear}
                title="Clear history"
                style={{
                  background: "none",
                  border: "none",
                  color: "#475569",
                  cursor: "pointer",
                  padding: 4,
                  display: "flex",
                  alignItems: "center",
                }}
              >
                <Trash2 size={13} />
              </button>
            )}
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
        </div>

        {/* Model logs popover — dev only */}
        {import.meta.env.DEV && showPromptStats && promptStats && (
          <div
            style={{
              position: "absolute",
              top: 45,
              right: 8,
              zIndex: 10,
              background: "#0f172a",
              border: "1px solid #1e293b",
              borderRadius: 8,
              padding: "12px 14px",
              width: 340,
              boxShadow: "0 8px 24px rgba(0,0,0,0.5)",
              fontSize: 12,
              color: "#94a3b8",
              maxHeight: 480,
              overflowY: "auto",
            }}
          >
            <div style={{ fontWeight: 600, color: "#e2e8f0", marginBottom: 8, fontSize: 11, textTransform: "uppercase", letterSpacing: "0.08em" }}>
              Model logs
            </div>

            {/* Per-command */}
            {promptStats.byCommand.length > 0 && (
              <div style={{ marginBottom: 10 }}>
                <div style={{ fontSize: 10, color: "#475569", marginBottom: 4 }}>By command</div>
                <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto auto", gap: "3px 8px", alignItems: "center" }}>
                  <span style={{ color: "#64748b", fontSize: 10 }}>command</span>
                  <span style={{ color: "#64748b", fontSize: 10, textAlign: "right" }}>calls</span>
                  <span style={{ color: "#64748b", fontSize: 10, textAlign: "right" }}>ok%</span>
                  <span style={{ color: "#64748b", fontSize: 10, textAlign: "right" }}>ms</span>
                  {promptStats.byCommand.map((c, i) => (
                    <>
                      <span key={`cmd-${i}`} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "#94a3b8" }}
                        title={`version: ${c.latestPromptVersion}`}>
                        {c.command}
                      </span>
                      <span key={`calls-${i}`} style={{ color: "#f1f5f9", textAlign: "right" }}>{c.totalCalls}</span>
                      <span key={`ok-${i}`} style={{ color: c.successRate < 80 ? "#f87171" : "#34d399", textAlign: "right" }}>{c.successRate}%</span>
                      <span key={`ms-${i}`} style={{ color: "#f1f5f9", textAlign: "right" }}>{c.avgLatencyMs}</span>
                    </>
                  ))}
                </div>
              </div>
            )}

            {/* Per-model */}
            {promptStats.byModel.length > 0 && (
              <div style={{ marginBottom: 10, borderTop: "1px solid #1e293b", paddingTop: 8 }}>
                <div style={{ fontSize: 10, color: "#475569", marginBottom: 4 }}>By model</div>
                <div style={{ display: "grid", gridTemplateColumns: "1fr auto auto", gap: "3px 8px", alignItems: "center" }}>
                  <span style={{ color: "#64748b", fontSize: 10 }}>model</span>
                  <span style={{ color: "#64748b", fontSize: 10, textAlign: "right" }}>calls</span>
                  <span style={{ color: "#64748b", fontSize: 10, textAlign: "right" }}>err%</span>
                  {promptStats.byModel.map((m, i) => (
                    <>
                      <span key={`mod-${i}`} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "#94a3b8", fontSize: 11 }}>
                        {m.model.split("/").pop()}
                      </span>
                      <span key={`mcalls-${i}`} style={{ color: "#f1f5f9", textAlign: "right" }}>{m.totalCalls}</span>
                      <span key={`merr-${i}`} style={{ color: m.errorRate > 10 ? "#f87171" : "#f1f5f9", textAlign: "right" }}>{m.errorRate}%</span>
                    </>
                  ))}
                </div>
              </div>
            )}

            {/* Recent errors */}
            {promptStats.recentErrors.length > 0 && (
              <div style={{ borderTop: "1px solid #1e293b", paddingTop: 8 }}>
                <div style={{ fontSize: 10, color: "#475569", marginBottom: 4 }}>Recent errors</div>
                {promptStats.recentErrors.map((e, i) => (
                  <div key={i} style={{ marginBottom: 6, fontSize: 10 }}>
                    <div style={{ color: "#f87171", fontWeight: 600 }}>{e.command}</div>
                    <div style={{ color: "#64748b", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{e.error}</div>
                  </div>
                ))}
              </div>
            )}

            {promptStats.byCommand.length === 0 && (
              <div style={{ color: "#475569", fontSize: 11 }}>No prompt logs yet — generate a tree to start logging.</div>
            )}
          </div>
        )}

        {/* Retrieval stats popover — dev only */}
        {import.meta.env.DEV && showStats && stats && (
          <div
            style={{
              position: "absolute",
              top: 45,
              right: 8,
              zIndex: 10,
              background: "#0f172a",
              border: "1px solid #1e293b",
              borderRadius: 8,
              padding: "12px 14px",
              width: 320,
              boxShadow: "0 8px 24px rgba(0,0,0,0.5)",
              fontSize: 12,
              color: "#94a3b8",
            }}
          >
            <div style={{ fontWeight: 600, color: "#e2e8f0", marginBottom: 8, fontSize: 11, textTransform: "uppercase", letterSpacing: "0.08em" }}>
              Retrieval stats
            </div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr auto", gap: "4px 12px", alignItems: "center" }}>
              <span>Total queries</span>
              <span style={{ color: "#f1f5f9", textAlign: "right" }}>{stats.totalQueries}</span>
              <span>Avg candidates (pre-rerank)</span>
              <span style={{ color: "#f1f5f9", textAlign: "right" }}>{stats.avgCandidatesBeforeRerank}</span>
              <span>Avg candidates (post-rerank)</span>
              <span style={{ color: "#f1f5f9", textAlign: "right" }}>{stats.avgCandidatesAfterRerank}</span>
              <span>Rerank fallback rate</span>
              <span style={{ color: stats.rerankFallbackRate > 25 ? "#f87171" : "#f1f5f9", textAlign: "right" }}>{stats.rerankFallbackRate}%</span>
              <span>Avg prematch chunks</span>
              <span style={{ color: "#f1f5f9", textAlign: "right" }}>{stats.avgPrematchChunksUsed}</span>
            </div>
            {stats.topQueriedNodes.length > 0 && (
              <div style={{ marginTop: 10, borderTop: "1px solid #1e293b", paddingTop: 8 }}>
                <div style={{ fontSize: 10, color: "#475569", marginBottom: 4 }}>Top queried nodes</div>
                {stats.topQueriedNodes.map((n, i) => (
                  <div key={i} style={{ display: "flex", justifyContent: "space-between", fontSize: 11, padding: "2px 0", color: "#64748b" }}>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 220 }}>{n.nodeId}</span>
                    <span style={{ color: "#94a3b8", flexShrink: 0 }}>{n.queryCount}×</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

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
            <div style={{ display: "flex", flexDirection: "column", gap: 12, marginTop: 20 }}>
              {nodeContext && nodeContext.matchedResources.length > 0 ? (
                <div>
                  <div style={{ fontSize: 10, color: "#334155", marginBottom: 6, textTransform: "uppercase", letterSpacing: "0.08em" }}>
                    Matched resources for this quest
                  </div>
                  <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                    {nodeContext.matchedResources.map((r, i) => {
                      const pageLabel = r.matchedPageStart != null
                        ? r.matchedPageEnd != null && r.matchedPageEnd !== r.matchedPageStart
                          ? `pp. ${r.matchedPageStart}–${r.matchedPageEnd}`
                          : `p. ${r.matchedPageStart}`
                        : null;
                      const inner = (
                        <div style={{ padding: "5px 8px", background: "#1e293b", border: "1px solid #334155", borderRadius: 6, cursor: r.url ? "pointer" : "default" }}>
                          <div style={{ display: "flex", alignItems: "center", gap: 4, color: "#94a3b8", fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                            {r.url && <ExternalLink size={9} style={{ flexShrink: 0 }} />}
                            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{r.title}</span>
                          </div>
                          {(r.matchedSectionTitle || pageLabel) && (
                            <div style={{ color: "#475569", fontSize: 10, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                              {[r.matchedSectionTitle, pageLabel].filter(Boolean).join(" · ")}
                            </div>
                          )}
                        </div>
                      );
                      return r.url ? (
                        <a key={i} href={r.url} target="_blank" rel="noopener noreferrer" style={{ textDecoration: "none" }}>
                          {inner}
                        </a>
                      ) : (
                        <div key={i}>{inner}</div>
                      );
                    })}
                  </div>
                </div>
              ) : (
                <div style={{ textAlign: "center", color: "#475569", fontSize: 12, marginTop: 20, lineHeight: 1.6 }}>
                  Ask Mimir about your resources.
                  <br />
                  Answers are grounded in your personal library.
                </div>
              )}
            </div>
          )}

          {resumed && messages.length > 0 && (
            <div
              style={{
                textAlign: "center",
                color: "#334155",
                fontSize: 10,
                padding: "2px 0 6px",
                borderBottom: "1px solid #1e293b",
                marginBottom: 4,
              }}
            >
              Conversation resumed
            </div>
          )}

          {messages.map((msg, i) => (
            <div key={i}>
              {i === 0 && msg.createdAt && (
                <div
                  style={{
                    textAlign: "center",
                    color: "#334155",
                    fontSize: 9,
                    marginBottom: 6,
                  }}
                >
                  {new Date(msg.createdAt).toLocaleString(undefined, {
                    month: "short",
                    day: "numeric",
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </div>
              )}
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
  const pageLabel = source.pageStart != null
    ? source.pageEnd != null && source.pageEnd !== source.pageStart
      ? `pp. ${source.pageStart}–${source.pageEnd}`
      : `p. ${source.pageStart}`
    : null;

  const inner = (
    <span
      style={{
        display: "inline-flex",
        flexDirection: "column",
        padding: "3px 8px",
        borderRadius: 6,
        background: "#1e293b",
        border: "1px solid #334155",
        cursor: source.url ? "pointer" : "default",
        maxWidth: 220,
      }}
      title={`${source.title} (${Math.round(source.score * 100)}% match)\n${source.chunk}`}
    >
      <span
        style={{
          display: "flex",
          alignItems: "center",
          gap: 3,
          color: "#94a3b8",
          fontSize: 10,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {source.title}
        <span style={{ color: "#475569" }}>{Math.round(source.score * 100)}%</span>
        {source.url && <ExternalLink size={9} />}
      </span>
      {(source.sectionTitle || pageLabel) && (
        <span
          style={{
            color: "#475569",
            fontSize: 9,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {[source.sectionTitle, pageLabel].filter(Boolean).join(" · ")}
        </span>
      )}
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
        {inner}
      </a>
    );
  }

  return inner;
}

// src/components/MimirChat.tsx
import { useState, useRef, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useLocation, useNavigate } from "react-router-dom";
import { emit, listen } from "@tauri-apps/api/event";
import { Send, X, Loader2, ExternalLink, Trash2, BarChart2, Cpu, CheckCircle, ArrowRight, HelpCircle, Plus, Search } from "lucide-react";
import { useMimirContext } from "../contexts/MimirContext";
import { validateOrLog, MimirChatResponseSchema } from "../lib/validators";
import type { NodeChatContext, Suggestion } from "../types";
import { HitlConfirmation, type ActionProposal } from "./HitlConfirmation";
import { ToolCallBlock } from "./ui/ToolCallBlock";
import { SourceCard } from "./ui/SourceCard";
import { tokenizeCitations } from "../lib/citations";
import type { ToolStatus } from "./ui/StatusBadge";

interface Source {
  title: string;
  url: string | null;
  chunk: string;
  score: number;
  sectionTitle: string | null;
  pageStart: number | null;
  pageEnd: number | null;
}

interface LiveToolCall {
  callIndex: number;
  toolName: string;
  status: ToolStatus;
  input?: unknown;
  output?: string;
  durationMs?: number;
}

interface ChatMessage {
  role: "user" | "mimir";
  content: string;
  sources?: Source[];
  suggestions?: Suggestion[];
  createdAt?: string;
  pendingApproval?: ActionProposal | null;
  toolCalls?: LiveToolCall[];
  reasoning?: string;
  /** Per-turn id used to route live ygg-agent-tool/ygg-agent-think events to this message while it's in flight. */
  turnId?: string;
}

interface StoredChatMessage {
  id: string;
  role: string;
  content: string;
  sources: Source[] | null;
  toolCalls?: LiveToolCall[] | null;
  reasoning?: string | null;
  createdAt: string;
}

interface MimirChatResponse {
  answer: string;
  sources: Source[];
  suggestions: Suggestion[];
  pendingApproval?: ActionProposal | null;
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
  onOpenCoverage,
}: {
  open: boolean;
  onToggle: () => void;
  onOpenCoverage?: () => void;
}) {
  const location = useLocation();
  const navigate = useNavigate();
  const { treeId, nodeId, nodeTitle, projectName, treeName, projectRootId, projectLabel, mode, setMimirContext } = useMimirContext();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [resumed, setResumed] = useState(false);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [showStats, setShowStats] = useState(false);
  const [stats, setStats] = useState<RetrievalStats | null>(null);
  const [showPromptStats, setShowPromptStats] = useState(false);
  const [promptStats, setPromptStats] = useState<PromptStats | null>(null);
  const [nodeContext, setNodeContext] = useState<NodeChatContext | null>(null);
  const [suggestionTapped, setSuggestionTapped] = useState(false);
  const [shouldAutoSend, setShouldAutoSend] = useState(false);
  const [scanStatus, setScanStatus] = useState<string | null>(null);
  const [coverage, setCoverage] = useState<{ covered: number; partial: number; gap: number; total: number } | null>(null);
  const [projectRoots, setProjectRoots] = useState<{ id: string; label: string; path: string }[]>([]);
  // Agent-owned working plan (Task 5) — tracked live from ygg-agent-tool set_plan/
  // update_plan events, cleared on finalize_plan. Persists across turns within a
  // session (not reset on send); reset when the session/context changes since we
  // don't hydrate a prior plan from the DB on load.
  const [plan, setPlan] = useState<string | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const loadedSessionKey = useRef<string | null>(null);
  // Refs so the (once-mounted) scan listener can read the current context.
  const treeIdRef = useRef(treeId);
  const nodeIdRef = useRef(nodeId);
  useEffect(() => { treeIdRef.current = treeId; nodeIdRef.current = nodeId; }, [treeId, nodeId]);

  // Clear a stale coverage chip when the tree/node/project context changes.
  useEffect(() => { setCoverage(null); }, [treeId, nodeId, projectRootId]);

  // Auto-scroll to bottom on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Auto-send for explain_prereq suggestion
  useEffect(() => {
    if (!shouldAutoSend) return;
    setShouldAutoSend(false);
    handleSend();
  }, [shouldAutoSend]); // handleSend excluded — called after input is set

  // Focus textarea when panel opens
  useEffect(() => {
    if (open) {
      setTimeout(() => textareaRef.current?.focus(), 200);
    }
  }, [open]);

  // Fetch registered project roots for the project picker
  useEffect(() => {
    (async () => {
      try {
        const roots = await invoke<{ id: string; label: string; path: string }[]>("get_project_roots_cmd");
        setProjectRoots(roots);
      } catch { /* best-effort */ }
    })();
  }, []);

  // Listen for background scan progress and completion events
  useEffect(() => {
    let unsubP: (() => void) | undefined;
    let unsubC: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const p = await listen<{ status: string; path: string }>("ygg-scan-progress", ({ payload }) => {
        setScanStatus(`Scanning ${payload.path}…`);
      });
      const c = await listen<{ treeId?: string; nodeId?: string | null; project?: string; filesScanned?: number; nodesAdded?: number; nodesEnriched?: number; edgesAdded?: number; error?: string }>(
        "ygg-scan-complete",
        ({ payload }) => {
          setScanStatus(null);
          const proj = payload.project ? `${payload.project}: ` : "";
          const summary = payload.error
            ? `🗂️ Scan of ${payload.project ?? "project"} failed: ${payload.error}`
            : `🗂️ Scan complete — ${proj}${payload.filesScanned ?? 0} files scanned, ${payload.nodesAdded ?? 0} new concepts, ${payload.nodesEnriched ?? 0} refreshed, ${payload.edgesAdded ?? 0} links.`;
          // The result is persisted to this node's chat session server-side (so the
          // agent sees it next turn). Mirror it into the open panel when it matches
          // the current context so it "pops up" immediately.
          const sameCtx = payload.treeId === treeIdRef.current && (payload.nodeId ?? null) === (nodeIdRef.current ?? null);
          if (sameCtx) {
            setMessages((prev) => [...prev, { role: "mimir", content: summary, createdAt: new Date().toISOString() }]);
          }
        }
      );
      if (cancelled) { p(); c(); return; }
      unsubP = p; unsubC = c;
    })();
    return () => { cancelled = true; unsubP?.(); unsubC?.(); };
  }, []);

  // Listen for the tree-vs-repo coverage pass (auto-run inline after a scan
  // completes; also fired by a manual re-run). Same cancel-guarded pattern as
  // the scan listener above — a bare async effect would double-subscribe
  // under StrictMode.
  useEffect(() => {
    let unsub: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const u = await listen<{ projectRootId?: string; treeId?: string; covered: number; partial: number; gap: number; total: number }>(
        "ygg-project-coverage",
        ({ payload }) => {
          if (payload.treeId !== treeIdRef.current) return;
          setCoverage({ covered: payload.covered, partial: payload.partial, gap: payload.gap, total: payload.total });
        }
      );
      if (cancelled) { u(); return; }
      unsub = u;
    })();
    return () => { cancelled = true; unsub?.(); };
  }, []);

  // Load session history when panel opens or context changes.
  // Tree-precedence: a selected tree node always wins over the project picker
  // (preserves today's behavior exactly); project scope only kicks in when
  // there's no node selected.
  const sessionKey = treeId && nodeId ? `${treeId}:${nodeId}` : (projectRootId ? `proj:${projectRootId}` : null);
  const loadSession = useCallback(async () => {
    if (treeId && nodeId) {
      if (loadedSessionKey.current === sessionKey) return;
      loadedSessionKey.current = sessionKey;

      // Fetch chat history and node context in parallel
      const [storedResult, ctxResult] = await Promise.allSettled([
        invoke<StoredChatMessage[]>("get_chat_session", { treeId, nodeId, projectRootId: null }),
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
          toolCalls: m.toolCalls ?? undefined,
          reasoning: m.reasoning ?? undefined,
        }));
        setMessages(loaded);
        setResumed(true);
      } else {
        setMessages([]);
        setResumed(false);
      }
      setPlan(null);
      return;
    }

    if (projectRootId) {
      if (loadedSessionKey.current === sessionKey) return;
      loadedSessionKey.current = sessionKey;
      setNodeContext(null);

      const storedResult = await invoke<StoredChatMessage[]>("get_chat_session", {
        treeId: null,
        nodeId: null,
        projectRootId,
      }).catch(() => null);

      if (storedResult && storedResult.length > 0) {
        const loaded: ChatMessage[] = storedResult.map((m) => ({
          role: m.role === "user" ? "user" : "mimir",
          content: m.content,
          sources: m.sources ?? undefined,
          createdAt: m.createdAt,
          toolCalls: m.toolCalls ?? undefined,
          reasoning: m.reasoning ?? undefined,
        }));
        setMessages(loaded);
        setResumed(true);
      } else {
        setMessages([]);
        setResumed(false);
      }
      setPlan(null);
      return;
    }

    setMessages([]);
    setResumed(false);
    setNodeContext(null);
    setPlan(null);
    loadedSessionKey.current = null;
  }, [treeId, nodeId, projectRootId, sessionKey]);

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
    const hasTreeScope = !!(treeId && nodeId);
    if (!hasTreeScope && !projectRootId) return;
    try {
      await invoke("clear_chat_session", {
        treeId: hasTreeScope ? treeId : null,
        nodeId: hasTreeScope ? nodeId : null,
        projectRootId: hasTreeScope ? null : projectRootId,
      });
    } catch { /* best-effort */ }
    setMessages([]);
    setResumed(false);
    setPlan(null);
    loadedSessionKey.current = null;
    // Keep nodeContext — resources are still relevant after clearing chat
  }

  // Derive current page context from route
  const page = location.pathname === "/" ? "home" : location.pathname.slice(1);

  async function handleSend() {
    const msg = input.trim();
    if (!msg || loading) return;

    const turnId = crypto.randomUUID();

    setInput("");
    setMessages((prev) => [
      ...prev,
      { role: "user", content: msg },
      { role: "mimir", content: "", turnId, toolCalls: [] },
    ]);
    setLoading(true);

    // Live agent visibility — route ygg-agent-tool/ygg-agent-think events for this
    // turn onto the pending assistant message, matched by turnId (not array index,
    // which can go stale if setMessages runs between updates).
    const unlistenTool = await listen<LiveToolCall & { turnId: string }>("ygg-agent-tool", ({ payload }) => {
      if (payload.turnId !== turnId) return;
      setMessages((prev) => {
        const idx = prev.findIndex((m) => m.turnId === turnId);
        if (idx === -1) return prev;
        const target = prev[idx];
        const calls = [...(target.toolCalls ?? [])];
        const at = calls.findIndex((c) => c.callIndex === payload.callIndex);
        const existing = at >= 0 ? calls[at] : undefined;
        // The "running" event carries `input`; the "success"/"error" event that
        // follows does not. Fall back to the previously recorded value for any
        // field the current payload omits, so the done event doesn't clobber
        // input (or other fields) with `undefined` via object spread.
        const rec: LiveToolCall = {
          callIndex: payload.callIndex,
          toolName: payload.toolName,
          status: payload.status,
          input: payload.input ?? existing?.input,
          output: payload.output ?? existing?.output,
          durationMs: payload.durationMs ?? existing?.durationMs,
        };
        if (at >= 0) calls[at] = rec;
        else calls.push(rec);
        const next = [...prev];
        next[idx] = { ...target, toolCalls: calls };
        return next;
      });
      // Track the agent's working plan (Task 5) — set_plan/update_plan carry the
      // new plan text in `input.content` (present on the "running" event); clear
      // it entirely when the agent finalizes.
      if (payload.toolName === "set_plan" || payload.toolName === "update_plan") {
        const content = (payload.input as { content?: unknown } | undefined)?.content;
        if (typeof content === "string" && content.trim()) setPlan(content);
      } else if (payload.toolName === "finalize_plan") {
        setPlan(null);
      }
    });
    const unlistenThink = await listen<{ turnId: string; text: string }>("ygg-agent-think", ({ payload }) => {
      if (payload.turnId !== turnId) return;
      setMessages((prev) => {
        const idx = prev.findIndex((m) => m.turnId === turnId);
        if (idx === -1) return prev;
        const target = prev[idx];
        const next = [...prev];
        next[idx] = { ...target, reasoning: [target.reasoning, payload.text].filter(Boolean).join("\n\n") };
        return next;
      });
    });

    try {
      const hasTreeScope = !!(treeId && nodeId);
      const raw = await invoke("mimir_chat", {
        message: msg,
        page,
        treeId,
        nodeId,
        projectRootId: hasTreeScope ? null : projectRootId,
        mode,
        nodeTitle,
        projectName,
        treeName,
        turnId,
      });
      const response = validateOrLog(MimirChatResponseSchema, raw, 'mimir_chat') as MimirChatResponse;
      setResumed(false);
      setSuggestionTapped(false);

      setMessages((prev) => {
        const idx = prev.findIndex((m) => m.turnId === turnId);
        if (idx === -1) {
          return [
            ...prev,
            {
              role: "mimir",
              content: response.answer,
              sources: response.sources,
              suggestions: response.suggestions ?? [],
              pendingApproval: response.pendingApproval ?? null,
            },
          ];
        }
        const next = [...prev];
        next[idx] = {
          ...next[idx],
          content: response.answer,
          sources: response.sources,
          suggestions: response.suggestions ?? [],
          pendingApproval: response.pendingApproval ?? null,
        };
        return next;
      });
      unlistenTool();
      unlistenThink();
    } catch (e) {
      setMessages((prev) => {
        const idx = prev.findIndex((m) => m.turnId === turnId);
        if (idx === -1) {
          return [...prev, { role: "mimir", content: `Error: ${e}` }];
        }
        const next = [...prev];
        next[idx] = { ...next[idx], content: `Error: ${e}` };
        return next;
      });
      unlistenTool();
      unlistenThink();
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

  const handleSuggestion = useCallback(async (suggestion: Suggestion) => {
    setSuggestionTapped(true);

    switch (suggestion.action) {
      case "mark_complete": {
        const nid = (suggestion.payload?.action === "mark_complete" ? suggestion.payload.node_id : null) ?? nodeId;
        if (!nid) break;
        try {
          await invoke("update_tree_node", { nodeId: nid, progress: 100 });
          await emit("ygg-checkpoint-completed", { nodeId: nid });
        } catch { /* best-effort */ }
        break;
      }
      case "next_quest": {
        if (!nodeContext || nodeContext.siblings.length === 0) break;
        // siblings are titles; we can't navigate by title alone — send a pre-filled follow-up
        setInput(`Tell me about the next checkpoint: ${nodeContext.siblings[0]}`);
        break;
      }
      case "explain_prereq": {
        setInput("Can you explain the prerequisites for this concept?");
        setShouldAutoSend(true);
        break;
      }
      case "add_resource": {
        const url = suggestion.payload?.action === "add_resource" ? suggestion.payload.url : undefined;
        navigate("/resources" + (url ? `?prefill=${encodeURIComponent(url)}` : ""));
        break;
      }
      case "find_gaps": {
        const tid = treeId;
        await emit("ygg-open-coverage", { treeId: tid });
        onOpenCoverage?.();
        break;
      }
    }
  }, [nodeId, nodeContext, navigate, onOpenCoverage]);

  const handleApprovalResolved = useCallback((index: number, resultMessage: string) => {
    setMessages((prev) => {
      const updated = [...prev];
      if (updated[index]) {
        updated[index] = { ...updated[index], pendingApproval: null };
      }
      return [...updated, { role: "mimir", content: resultMessage }];
    });
  }, []);

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
              {!(treeId && nodeId) && projectRootId
                ? `scoped to ${projectLabel ?? "project"}`
                : "RAG assistant"}
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
            {((treeId && nodeId) || projectRootId) && messages.length > 0 && (
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

        {/* Project picker — scopes chat to a project session when no tree node is active */}
        <div
          style={{
            padding: "6px 16px",
            borderBottom: "1px solid #1e293b",
            display: "flex",
            alignItems: "center",
            gap: 8,
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: 10, color: "#475569", textTransform: "uppercase", letterSpacing: "0.06em", flexShrink: 0 }}>
            Project
          </span>
          <select
            value={projectRootId ?? ""}
            onChange={(e) => {
              const selectedId = e.target.value || null;
              const selected = selectedId ? projectRoots.find((r) => r.id === selectedId) ?? null : null;
              setMimirContext({ projectRootId: selectedId, projectLabel: selected?.label ?? null });
            }}
            title={treeId && nodeId ? "Chat is scoped to the active tree node; project scope applies when no node is selected." : undefined}
            style={{
              flex: 1,
              minWidth: 0,
              background: "#1e293b",
              border: "1px solid #334155",
              borderRadius: 6,
              color: "#e2e8f0",
              fontSize: 11,
              padding: "3px 6px",
              fontFamily: "inherit",
            }}
          >
            <option value="">— none —</option>
            {projectRoots.map((r) => (
              <option key={r.id} value={r.id}>{r.label}</option>
            ))}
          </select>
          <div style={{ display: "flex", gap: 4, flexShrink: 0 }}>
            <button
              onClick={() => setMimirContext({ mode: "chat" })}
              title="Chat mode — quick answers, smaller tool budget"
              style={{
                background: mode === "chat" ? "rgba(16,185,129,0.15)" : "none",
                border: mode === "chat" ? "1px solid rgba(16,185,129,0.4)" : "1px solid #334155",
                color: mode === "chat" ? "#10b981" : "#64748b",
                cursor: "pointer",
                padding: "3px 8px",
                borderRadius: 6,
                fontSize: 10,
                fontWeight: 600,
              }}
            >
              Chat
            </button>
            <button
              onClick={() => setMimirContext({ mode: "dive" })}
              title="Dive mode — deeper exploration, bigger tool budget"
              style={{
                background: mode === "dive" ? "rgba(16,185,129,0.15)" : "none",
                border: mode === "dive" ? "1px solid rgba(16,185,129,0.4)" : "1px solid #334155",
                color: mode === "dive" ? "#10b981" : "#64748b",
                cursor: "pointer",
                padding: "3px 8px",
                borderRadius: 6,
                fontSize: 10,
                fontWeight: 600,
              }}
            >
              Dive
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

        {/* Agent-owned working plan (Task 5) — persists above the message list
            for the life of the session; collapsible, styled like ToolCallBlock. */}
        {plan && (
          <div style={{ padding: "0 14px", paddingTop: 8 }}>
            <details className="rounded-md border text-sm border-white/10 bg-white/5">
              <summary className="cursor-pointer select-none px-2.5 py-1.5 flex items-center gap-2">
                <span>📋</span>
                <span className="font-mono text-xs">Plan</span>
              </summary>
              <pre className="px-3 pb-2 text-xs opacity-80 whitespace-pre-wrap break-words">{plan}</pre>
            </details>
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

          {(() => {
            // Index of last mimir message — only that one shows chips
            let lastMimirIdx = -1;
            for (let k = messages.length - 1; k >= 0; k--) {
              if (messages[k].role === "mimir") { lastMimirIdx = k; break; }
            }
            return messages.map((msg, i) => (
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
                    {/* Collapsed-by-default agent reasoning trace */}
                    {msg.reasoning && (
                      <details className="mb-2 text-xs opacity-60">
                        <summary className="cursor-pointer select-none">🤔 Thinking</summary>
                        <pre className="mt-1 whitespace-pre-wrap break-words">{msg.reasoning}</pre>
                      </details>
                    )}
                    {/* Live/persisted tool-call trace */}
                    {msg.toolCalls?.map((tc) => (
                      <div className="mb-1" key={tc.callIndex}>
                        <ToolCallBlock
                          toolName={tc.toolName}
                          status={tc.status}
                          input={tc.input}
                          output={tc.output}
                          durationMs={tc.durationMs}
                        />
                      </div>
                    ))}
                    {tokenizeCitations(msg.content).map((t, k) =>
                      t.kind === "text" ? (
                        <span key={k}>{t.text}</span>
                      ) : (
                        <a
                          key={k}
                          href={msg.sources?.[t.index - 1]?.url ?? undefined}
                          target="_blank"
                          rel="noreferrer"
                          className="text-blue-400"
                        >
                          [{t.index}]
                        </a>
                      )
                    )}
                  </div>
                  {/* HITL confirmation card for destructive action proposals */}
                  {msg.pendingApproval && (
                    <HitlConfirmation
                      proposal={msg.pendingApproval}
                      treeId={treeId ?? null}
                      onResolved={(resultMessage) => handleApprovalResolved(i, resultMessage)}
                    />
                  )}
                  {/* Source cards */}
                  {msg.sources && msg.sources.length > 0 && (
                    <div className="mt-2 space-y-1">
                      {msg.sources.map((src, j) => (
                        <SourceCard key={j} index={j + 1} title={src.title} url={src.url} snippet={src.chunk} />
                      ))}
                    </div>
                  )}
                  {/* Suggestion chips — only on last mimir message, disappear after tap */}
                  {i === lastMimirIdx && !suggestionTapped && msg.suggestions && msg.suggestions.length > 0 && !loading && (
                    <div style={{ display: "flex", flexWrap: "wrap", gap: 5, marginTop: 8 }}>
                      {msg.suggestions.map((sug, j) => {
                        const chipMeta: Record<string, { icon: React.ReactNode; color: string; bg: string; border: string }> = {
                          mark_complete: {
                            icon: <CheckCircle size={11} />,
                            color: "#6ee7b7",
                            bg: "rgba(16,185,129,0.1)",
                            border: "rgba(16,185,129,0.3)",
                          },
                          next_quest: {
                            icon: <ArrowRight size={11} />,
                            color: "#93c5fd",
                            bg: "rgba(59,130,246,0.1)",
                            border: "rgba(59,130,246,0.3)",
                          },
                          explain_prereq: {
                            icon: <HelpCircle size={11} />,
                            color: "#fcd34d",
                            bg: "rgba(245,158,11,0.1)",
                            border: "rgba(245,158,11,0.3)",
                          },
                          add_resource: {
                            icon: <Plus size={11} />,
                            color: "#c4b5fd",
                            bg: "rgba(139,92,246,0.1)",
                            border: "rgba(139,92,246,0.3)",
                          },
                          find_gaps: {
                            icon: <Search size={11} />,
                            color: "#94a3b8",
                            bg: "rgba(100,116,139,0.1)",
                            border: "rgba(100,116,139,0.3)",
                          },
                        };
                        const meta = chipMeta[sug.action] ?? chipMeta["find_gaps"];
                        return (
                          <button
                            key={j}
                            onClick={() => handleSuggestion(sug)}
                            style={{
                              display: "flex", alignItems: "center", gap: 4,
                              padding: "4px 9px", borderRadius: 20, fontSize: 11,
                              background: meta.bg,
                              border: `1px solid ${meta.border}`,
                              color: meta.color,
                              cursor: "pointer", fontFamily: "inherit",
                              transition: "opacity 0.15s",
                            }}
                            onMouseEnter={e => (e.currentTarget.style.opacity = "0.75")}
                            onMouseLeave={e => (e.currentTarget.style.opacity = "1")}
                          >
                            {meta.icon}
                            {sug.label}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>
            ));
          })()}

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
          {scanStatus && (
            <div className="px-3 py-1 text-xs opacity-70 border-t border-white/10">🗂️ {scanStatus}</div>
          )}
          {coverage && (
            <div className="px-3 py-1 text-xs opacity-70 border-t border-white/10">
              🧭 {coverage.covered}/{coverage.total} covered · {coverage.partial} partial · {coverage.gap} gaps
            </div>
          )}
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


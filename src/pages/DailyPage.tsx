import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, Search, Trash2, GitBranch } from "lucide-react";
import type { CheckpointData, DailyLog, DailyQuestLink } from "../types";

type Quadrant = "do" | "schedule" | "delegate" | "eliminate";

function quadrantFor(important: boolean, urgent: boolean): Quadrant {
  if (important && urgent) return "do";
  if (important && !urgent) return "schedule";
  if (!important && urgent) return "delegate";
  return "eliminate";
}

// Each quadrant: accent color, axis description
const Q_META: Record<Quadrant, { color: string; dim: string }> = {
  do:       { color: "#ef4444", dim: "#ef444420" },
  schedule: { color: "#3b82f6", dim: "#3b82f620" },
  delegate: { color: "#f59e0b", dim: "#f59e0b20" },
  eliminate:{ color: "#334155", dim: "#33415520" },
};

const Q_ORDER: Quadrant[] = ["do", "schedule", "delegate", "eliminate"];

interface QuestRow {
  id: string; treeId: string; projectId: string; projectName: string;
  treeName: string; title: string; description: string; progress: number;
  tasks: unknown; orderIndex: number;
}

function getCheckpoint(tasks: unknown): CheckpointData | null {
  if (!tasks) return null;
  if (Array.isArray(tasks)) {
    const t = tasks[0]; if (!t) return null;
    return { mastery_criteria: t.description ?? "", exercises: [], notes: t.notes ?? "", completed: t.completed ?? false };
  }
  return tasks as CheckpointData;
}

function fmt(d: Date) {
  return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,"0")}-${String(d.getDate()).padStart(2,"0")}`;
}

function isToday(d: Date) { return fmt(d) === fmt(new Date()); }
function isPast(d: Date)  { return fmt(d) < fmt(new Date()); }

function displayDate(d: Date) {
  if (isToday(d)) return "TODAY";
  const diff = Math.round((new Date(fmt(d)).getTime() - new Date(fmt(new Date())).getTime()) / 86400000);
  if (diff === -1) return "YESTERDAY";
  if (diff === 1)  return "TOMORROW";
  return d.toLocaleDateString("en-US", { weekday:"short", month:"short", day:"numeric" }).toUpperCase();
}

// ─── Quest Picker ─────────────────────────────────────────────────────────────

function QuestPicker({ onSelect, onClose }: { onSelect:(q:QuestRow)=>void; onClose:()=>void }) {
  const [quests, setQuests] = useState<QuestRow[]>([]);
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    invoke<QuestRow[]>("get_all_quests")
      .then(q => setQuests(q.filter(n => n.progress < 100)))
      .catch(console.error);
    setTimeout(() => inputRef.current?.focus(), 50);
  }, []);

  const filtered = query.trim()
    ? quests.filter(q => q.title.toLowerCase().includes(query.toLowerCase()) || q.projectName.toLowerCase().includes(query.toLowerCase()))
    : quests;

  return (
    <div className="absolute left-0 top-full mt-1 z-50 w-96 bg-[#080c12] border border-[#1e293b] shadow-2xl overflow-hidden" style={{ fontFamily: "'Space Mono', monospace", borderTop: "2px solid #3b82f6" }}>
      <style>{`
        .qpick-border { border-top: 2px solid #3b82f6; }
        .qpick-item:hover { background: #0f172a; }
      `}</style>
      <div className="qpick-border relative border border-[#1e293b]">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3 h-3 text-[#3b82f6]" />
        <input ref={inputRef} type="text" value={query}
          onChange={e => setQuery(e.target.value)}
          onKeyDown={e => e.key === "Escape" && onClose()}
          placeholder="/ SEARCH QUESTS"
          className="w-full pl-9 pr-3 py-2.5 bg-transparent text-xs text-slate-300 placeholder-[#1e4a7a] outline-none border-b border-[#1e293b] tracking-widest"
        />
      </div>
      <div className="max-h-60 overflow-y-auto">
        {filtered.length === 0
          ? <p className="text-[10px] text-[#334155] text-center py-5 tracking-widest">NO INCOMPLETE QUESTS</p>
          : filtered.slice(0, 20).map(q => (
              <button key={q.id} onClick={() => onSelect(q)}
                className="qpick-item w-full text-left px-3 py-2.5 border-b border-[#0f172a] transition-colors">
                <div className="text-xs text-slate-300 truncate">{q.title}</div>
                <div className="text-[10px] text-[#475569] truncate tracking-wider mt-0.5">
                  {q.projectName.toUpperCase()} · {q.treeName.toUpperCase()}
                </div>
              </button>
            ))
        }
      </div>
    </div>
  );
}

// ─── Task Row ─────────────────────────────────────────────────────────────────

function TaskRow({ link, readOnly, toggling, onToggle, onRemove, onMove }: {
  link: DailyQuestLink; readOnly: boolean; toggling: boolean;
  onToggle:()=>void; onRemove:()=>void; onMove:(q:Quadrant)=>void;
}) {
  const [moveOpen, setMoveOpen] = useState(false);
  const isLinked = !!link.nodeId;
  // Completion: use persisted `completed` flag for both free-text and quest tasks.
  // Quest task progress is still synced to the tree but we don't use it for display here.
  const isDone   = link.completed || (isLinked && link.nodeProgress === 100);
  const isLocked = link.nodeIsLocked === true;
  const title    = isLinked ? (link.nodeTitle ?? "Untitled") : (link.freeText ?? "");
  const accent   = Q_META[link.quadrant as Quadrant].color;

  return (
    <div className={`group relative flex items-center gap-2.5 px-3 py-2 border-b border-[#0d1117] transition-all ${isDone ? "opacity-35" : ""}`}
      style={{ borderLeft: `2px solid ${isDone ? "#1a2535" : accent}` }}
    >
      {/* Checkbox — clicking it or the title text toggles done */}
      <button onClick={onToggle} disabled={toggling || readOnly || isLocked}
        className={`flex-shrink-0 w-3.5 h-3.5 border flex items-center justify-center transition-all ${
          isDone ? "border-[#1e293b] bg-[#1e293b]"
          : isLocked ? "border-[#1e293b] cursor-not-allowed"
          : "border-[#334155] hover:border-[#475569] cursor-pointer"
        } ${toggling ? "opacity-40" : ""}`}
        style={{ borderRadius: 0 }}
      >
        {isDone && <div className="w-1.5 h-1.5 bg-[#334155]" />}
      </button>

      {/* Title — also clickable to toggle */}
      <span
        onClick={!readOnly && !isLocked ? onToggle : undefined}
        className={`flex-1 text-xs min-w-0 truncate ${
          isDone ? "line-through text-[#334155]" : "text-[#94a3b8]"
        } ${!readOnly && !isLocked ? "cursor-pointer" : ""}`}
        style={{ fontFamily: "'Space Mono', monospace", letterSpacing: "0.02em" }}
      >
        {title}
      </span>

      {/* Quest icon */}
      {isLinked && !isDone && (
        <GitBranch className="w-2.5 h-2.5 flex-shrink-0" style={{ color: accent, opacity: 0.4 }} />
      )}

      {/* Progress micro-bar (quest nodes only, not done) */}
      {isLinked && !isDone && link.nodeProgress != null && link.nodeProgress > 0 && (
        <div className="flex-shrink-0 w-8 h-0.5 bg-[#1e293b] overflow-hidden">
          <div className="h-full" style={{ width: `${link.nodeProgress}%`, backgroundColor: accent, opacity: 0.5 }} />
        </div>
      )}

      {/* Locked badge */}
      {isLocked && (
        <span className="text-[9px] text-amber-800 tracking-widest flex-shrink-0" style={{ fontFamily: "'Space Mono', monospace" }}>LOCK</span>
      )}

      {/* Hover actions — move + delete ONLY (no delete-on-click-title) */}
      {!readOnly && (
        <div className="hidden group-hover:flex items-center gap-0.5 flex-shrink-0">
          <div className="relative">
            <button onClick={() => setMoveOpen(v => !v)} onBlur={() => setTimeout(() => setMoveOpen(false), 150)}
              className="p-1 text-[#334155] hover:text-[#64748b] transition-colors text-[10px]"
              style={{ fontFamily: "'Space Mono', monospace" }} title="Move">→</button>
            {moveOpen && (
              <div className="absolute right-0 top-full mt-0.5 z-20 bg-[#080c12] border border-[#1e293b] shadow-xl py-1 min-w-[100px]">
                {Q_ORDER.filter(q => q !== link.quadrant).map(q => (
                  <button key={q} onMouseDown={e => e.preventDefault()}
                    onClick={() => { onMove(q); setMoveOpen(false); }}
                    className="w-full text-left px-3 py-1.5 text-[10px] tracking-widest hover:bg-[#0f172a] transition-colors"
                    style={{ fontFamily: "'Space Mono', monospace", color: Q_META[q].color }}>
                    {q.toUpperCase()}
                  </button>
                ))}
              </div>
            )}
          </div>
          {/* Delete is the ONLY way to remove — separate from completion */}
          <button onClick={onRemove} className="p-1 text-[#334155] hover:text-red-500 transition-colors" title="Delete">
            <Trash2 className="w-2.5 h-2.5" />
          </button>
        </div>
      )}
    </div>
  );
}

// ─── Main Page ────────────────────────────────────────────────────────────────

export default function DailyPage() {
  const [currentDate, setCurrentDate] = useState(new Date());
  const [log, setLog] = useState<DailyLog | null>(null);
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState<Set<string>>(new Set());
  const [notes, setNotes] = useState("");
  const [toast, setToast] = useState<string | null>(null);
  const [taskText, setTaskText] = useState("");
  const [isImportant, setIsImportant] = useState(true);
  const [isUrgent, setIsUrgent] = useState(true);
  const [showQuestPicker, setShowQuestPicker] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const readOnly = isPast(currentDate);
  const dateStr  = fmt(currentDate);
  const currentQ = quadrantFor(isImportant, isUrgent);

  const loadLog = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<DailyLog>("get_daily_log", { date: dateStr });
      setLog(data);
      setNotes(data.notes ?? "");
    } catch(e) { console.error(e); }
    finally    { setLoading(false); }
  }, [dateStr]);

  useEffect(() => { loadLog(); setTaskText(""); setShowQuestPicker(false); }, [loadLog]);

  function showToast(msg: string) { setToast(msg); setTimeout(() => setToast(null), 3200); }

  function shiftDate(d: number) {
    setCurrentDate(prev => { const n = new Date(prev); n.setDate(n.getDate()+d); return n; });
  }

  async function handleAddFreeTask() {
    const text = taskText.trim(); if (!text) return;
    try { await invoke("add_free_task_to_day", { date:dateStr, text, quadrant:currentQ }); setTaskText(""); await loadLog(); }
    catch(e) { console.error(e); }
  }

  async function handleAddQuest(quest: QuestRow) {
    setShowQuestPicker(false);
    try { await invoke("add_quest_to_day", { date:dateStr, nodeId:quest.id, quadrant:currentQ }); await loadLog(); }
    catch(e) { console.error(e); }
  }

  async function toggleCompletion(link: DailyQuestLink) {
    if (readOnly) return;
    setToggling(p => new Set(p).add(link.id));
    try {
      // Always flip the persisted completed flag on the daily_quest_links row
      await invoke("toggle_task_complete", { linkId: link.id });

      // For quest nodes, also sync progress to the tree
      if (link.nodeId) {
        const nd = await invoke<{ tasks:unknown; treeId:string; projectId:string }>("get_tree_node_data", { nodeId: link.nodeId });
        const cp = getCheckpoint(nd.tasks);
        if (!cp) { showToast("CHECKPOINT DATA MISSING"); }
        else {
          const newDone     = !link.completed && link.nodeProgress !== 100;
          const prevProg    = link.nodeProgress ?? 0;
          const newProgress = newDone ? 100 : (prevProg === 100 ? 0 : prevProg);
          await invoke("update_tree_node", { nodeId:link.nodeId, title:null, description:null, progress:newProgress, tasks:{...cp,completed:newDone}, resources:null, position:null });
          await invoke("recalculate_tree_progress", { treeId: nd.treeId });
          await invoke("update_project_progress",   { projectId: nd.projectId });
          invoke("sync_skills_from_trees").then(() => invoke("recalculate_skill_levels")).catch(console.warn);
        }
      }

      await loadLog();
    } catch(e) { console.error(e); }
    finally { setToggling(p => { const s=new Set(p); s.delete(link.id); return s; }); }
  }

  async function handleRemove(linkId: string) {
    if (readOnly) return;
    try { await invoke("remove_from_day", { linkId }); await loadLog(); } catch(e) { console.error(e); }
  }

  async function handleMove(linkId: string, quadrant: Quadrant) {
    if (readOnly) return;
    try { await invoke("move_to_quadrant", { linkId, quadrant }); await loadLog(); } catch(e) { console.error(e); }
  }

  async function saveNotes() {
    if (readOnly) return;
    try { await invoke("upsert_daily_notes", { date:dateStr, notes }); } catch(e) { console.error(e); }
  }

  function linksFor(q: Quadrant) {
    return (log?.links ?? []).filter(l => l.quadrant === q).sort((a,b) => a.sortOrder - b.sortOrder);
  }

  const totalToday = log?.links.length ?? 0;
  const doneToday  = (log?.links ?? []).filter(l => l.nodeProgress === 100).length;

  return (
    <>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Space+Mono:ital,wght@0,400;0,700;1,400&family=Inter:wght@400;500&display=swap');

        .dm-root { font-family: 'Inter', sans-serif; background: #060a0f; }
        .dm-mono  { font-family: 'Space Mono', monospace; }

        .dm-axis-v {
          writing-mode: vertical-rl;
          transform: rotate(180deg);
          letter-spacing: 0.18em;
          font-size: 9px;
          font-weight: 700;
        }

        .dm-cell { transition: border-color 0.15s; }
        .dm-cell:hover { border-color: #1e2d3d !important; }

        .dm-input::placeholder { color: #1e293b; }
        .dm-input:focus { outline: none; }

        .dm-check-pill {
          display: flex; align-items: center; gap: 5px;
          padding: 3px 8px;
          border: 1px solid #1e293b;
          cursor: pointer;
          transition: all 0.12s;
          font-size: 9px; letter-spacing: 0.12em; font-weight: 700;
          user-select: none;
        }
        .dm-check-pill.active-imp { border-color: #166534; color: #4ade80; background: #14532d18; }
        .dm-check-pill.active-urg { border-color: #991b1b; color: #f87171; background: #7f1d1d18; }
        .dm-check-pill.inactive   { color: #334155; }
        .dm-check-pill.inactive:hover { border-color: #334155; color: #64748b; }

        .dm-q-badge {
          font-size: 8px; letter-spacing: 0.2em; font-weight: 700; padding: 2px 8px;
          border: 1px solid; transition: all 0.12s;
        }

        .dm-quest-btn {
          font-size: 9px; letter-spacing: 0.14em; font-weight: 700;
          padding: 3px 10px; border: 1px solid #1e293b;
          color: #475569; display: flex; align-items: center; gap: 4px;
          transition: all 0.12s; cursor: pointer; background: transparent;
        }
        .dm-quest-btn:hover, .dm-quest-btn.active {
          border-color: #1d4ed8; color: #60a5fa; background: #1e3a5f18;
        }

        .dm-date-btn {
          padding: 4px 12px; border: 1px solid #1e293b;
          font-size: 11px; letter-spacing: 0.12em; font-weight: 700;
          background: transparent; color: #64748b; cursor: pointer;
          transition: all 0.15s;
        }
        .dm-date-btn.today { border-color: #334155; color: #94a3b8; }
        .dm-date-btn:hover { border-color: #475569; color: #cbd5e1; }

        .dm-arrow { padding: 4px 6px; background:transparent; border:none; cursor:pointer; color:#334155; transition:color 0.12s; }
        .dm-arrow:hover { color: #64748b; }

        .dm-notes-area {
          width: 100%; resize: none; outline: none;
          background: transparent; border: none; border-top: 1px solid #0d1117;
          padding: 10px 12px; font-size: 11px; color: #475569; line-height: 1.7;
          font-family: 'Space Mono', monospace;
        }
        .dm-notes-area::placeholder { color: #1e293b; }
        .dm-notes-area:focus { border-top-color: #1e293b; }

        .dm-toast {
          position: fixed; bottom: 24px; left: 50%; transform: translateX(-50%);
          background: #0f172a; border: 1px solid #334155;
          padding: 8px 20px; font-size: 10px; letter-spacing: 0.16em;
          color: #94a3b8; z-index: 100;
          font-family: 'Space Mono', monospace;
        }

        .dm-progress-bar {
          height: 1px; background: #0d1117;
          transition: width 0.4s ease;
        }
      `}</style>

      <div className="dm-root flex flex-col h-full min-h-0 overflow-hidden" style={{ color: "#64748b" }}>

        {/* ── Top bar ─────────────────────────────────────────────────────────── */}
        <div style={{ borderBottom: "1px solid #0d1117", padding: "10px 20px" }}
          className="flex items-center justify-between flex-shrink-0">

          {/* Left: title + progress */}
          <div className="flex items-baseline gap-4">
            <span className="dm-mono" style={{ fontSize: 11, letterSpacing: "0.24em", fontWeight: 700, color: "#e2e8f0" }}>
              DAILY MATRIX
            </span>
            {totalToday > 0 && (
              <span className="dm-mono" style={{ fontSize: 9, letterSpacing: "0.16em", color: "#334155" }}>
                {doneToday}/{totalToday} DONE
              </span>
            )}
            {readOnly && (
              <span className="dm-mono" style={{ fontSize: 9, letterSpacing: "0.18em", color: "#78350f" }}>
                READ-ONLY
              </span>
            )}
          </div>

          {/* Right: date nav */}
          <div className="flex items-center gap-1">
            <button className="dm-arrow" onClick={() => shiftDate(-1)}>
              <ChevronLeft style={{ width: 13, height: 13 }} />
            </button>
            <button className={`dm-date-btn dm-mono ${isToday(currentDate) ? "today" : ""}`}
              onClick={() => setCurrentDate(new Date())}>
              {displayDate(currentDate)}
            </button>
            <button className="dm-arrow" onClick={() => shiftDate(1)}>
              <ChevronRight style={{ width: 13, height: 13 }} />
            </button>
          </div>
        </div>

        {/* ── Progress strip ──────────────────────────────────────────────────── */}
        {totalToday > 0 && (
          <div style={{ background: "#0d1117", height: 2, flexShrink: 0 }}>
            <div className="dm-progress-bar" style={{ width: `${(doneToday/totalToday)*100}%`, background: "#166534" }} />
          </div>
        )}

        {/* ── Command input ────────────────────────────────────────────────────── */}
        {!readOnly && (
          <div style={{ borderBottom: "1px solid #0d1117", padding: "0 20px", flexShrink: 0 }}
            className="flex items-center gap-3">

            {/* $ prompt */}
            <span className="dm-mono flex-shrink-0" style={{ fontSize: 10, color: "#1e4a7a", letterSpacing: "0.1em" }}>$</span>

            {/* Input */}
            <div className="relative flex-1">
              <input ref={inputRef} type="text" value={taskText}
                onChange={e => setTaskText(e.target.value)}
                onKeyDown={e => { if (e.key === "Enter") handleAddFreeTask(); }}
                placeholder="new task…"
                className="dm-input dm-mono w-full bg-transparent py-3 text-xs"
                style={{ color: "#94a3b8", letterSpacing: "0.06em", fontSize: 11 }}
              />
              {showQuestPicker && (
                <QuestPicker onSelect={handleAddQuest} onClose={() => setShowQuestPicker(false)} />
              )}
            </div>

            {/* Axis toggles */}
            <div className="flex items-center gap-1.5 flex-shrink-0">
              <button className={`dm-check-pill dm-mono ${isImportant ? "active-imp" : "inactive"}`}
                onClick={() => setIsImportant(v => !v)}>
                <span style={{ width: 5, height: 5, borderRadius: "50%", background: "currentColor", display: "inline-block", flexShrink: 0 }} />
                IMP
              </button>
              <button className={`dm-check-pill dm-mono ${isUrgent ? "active-urg" : "inactive"}`}
                onClick={() => setIsUrgent(v => !v)}>
                <span style={{ width: 5, height: 5, borderRadius: "50%", background: "currentColor", display: "inline-block", flexShrink: 0 }} />
                URG
              </button>

              {/* Target quadrant indicator */}
              <span className="dm-q-badge dm-mono"
                style={{ color: Q_META[currentQ].color, borderColor: Q_META[currentQ].color + "40", background: Q_META[currentQ].dim }}>
                {currentQ.toUpperCase()}
              </span>

              {/* Quest picker */}
              <button className={`dm-quest-btn dm-mono ${showQuestPicker ? "active" : ""}`}
                onClick={() => setShowQuestPicker(v => !v)}>
                <GitBranch style={{ width: 10, height: 10 }} />
                QUEST
              </button>
            </div>
          </div>
        )}

        {/* ── Matrix ──────────────────────────────────────────────────────────── */}
        {loading ? (
          <div className="flex-1 flex items-center justify-center">
            <span className="dm-mono" style={{ fontSize: 9, letterSpacing: "0.3em", color: "#1e293b" }}>LOADING…</span>
          </div>
        ) : (
          <div className="flex-1 min-h-0 flex" style={{ padding: "0 20px 0 0" }}>

            {/* ── Row axis labels ──────────────────────────────────────────────── */}
            <div style={{ width: 28, flexShrink: 0, display: "flex", flexDirection: "column" }}>
              <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
                <span className="dm-axis-v dm-mono" style={{ color: "#4ade80", opacity: 0.7 }}>IMPORTANT</span>
              </div>
              <div style={{ height: 1, background: "#0d1117", margin: "0 8px" }} />
              <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center" }}>
                <span className="dm-axis-v dm-mono" style={{ color: "#334155" }}>NOT IMPORTANT</span>
              </div>
            </div>

            {/* ── Grid body ────────────────────────────────────────────────────── */}
            <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>

              {/* Column axis labels */}
              <div style={{ display: "flex", flexShrink: 0, paddingLeft: 4 }}>
                <div style={{ flex: 1, padding: "6px 12px", textAlign: "center" }}>
                  <span className="dm-mono" style={{ fontSize: 9, letterSpacing: "0.22em", color: "#f87171", opacity: 0.8 }}>URGENT</span>
                </div>
                <div style={{ width: 1, background: "#0d1117" }} />
                <div style={{ flex: 1, padding: "6px 12px", textAlign: "center" }}>
                  <span className="dm-mono" style={{ fontSize: 9, letterSpacing: "0.22em", color: "#334155" }}>NOT URGENT</span>
                </div>
              </div>

              {/* 2×2 cells */}
              <div style={{ flex: 1, minHeight: 0, display: "grid", gridTemplateColumns: "1fr 1fr", gridTemplateRows: "1fr 1fr", gap: 1, background: "#0d1117", paddingLeft: 4 }}>
                {Q_ORDER.map(q => {
                  const items  = linksFor(q);
                  const meta   = Q_META[q];
                  return (
                    <div key={q} className="dm-cell flex flex-col min-h-0 overflow-hidden"
                      style={{ background: "#060a0f", borderTop: `1px solid ${meta.color}40` }}>

                      {/* Cell header — count chip only, no quadrant name label */}
                      {items.length > 0 && (
                        <div style={{ padding: "3px 10px", borderBottom: "1px solid #0d1117", display: "flex", alignItems: "center", justifyContent: "flex-end", flexShrink: 0 }}>
                          <span className="dm-mono" style={{ fontSize: 8, color: meta.color, opacity: 0.45 }}>{items.length}</span>
                        </div>
                      )}

                      {/* Items */}
                      <div style={{ flex: 1, overflowY: "auto", minHeight: 0 }}>
                        {items.length === 0 ? (
                          <div style={{ display: "flex", alignItems: "center", justifyContent: "center", height: "100%", padding: "12px" }}>
                            <span className="dm-mono" style={{ fontSize: 8, color: "#1e293b", letterSpacing: "0.2em" }}>—</span>
                          </div>
                        ) : items.map(link => (
                          <TaskRow key={link.id} link={link} readOnly={readOnly}
                            toggling={toggling.has(link.id)}
                            onToggle={() => toggleCompletion(link)}
                            onRemove={() => handleRemove(link.id)}
                            onMove={(targetQ) => handleMove(link.id, targetQ)}
                          />
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        )}

        {/* ── Notes ───────────────────────────────────────────────────────────── */}
        <div style={{ flexShrink: 0, borderTop: "1px solid #0d1117" }}>
          <div style={{ padding: "4px 20px 0", display: "flex", alignItems: "center", gap: 8 }}>
            <span className="dm-mono" style={{ fontSize: 8, letterSpacing: "0.24em", color: "#1e293b" }}>NOTES</span>
          </div>
          <textarea value={notes} onChange={e => setNotes(e.target.value)}
            onBlur={saveNotes} disabled={readOnly}
            placeholder={readOnly ? "" : "// reflections, blockers, intentions…"}
            className="dm-notes-area"
            rows={2}
            style={{ opacity: readOnly ? 0.4 : 1 }}
          />
        </div>

        {/* ── Toast ───────────────────────────────────────────────────────────── */}
        {toast && <div className="dm-toast">{toast}</div>}
      </div>
    </>
  );
}

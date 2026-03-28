import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CalendarDays, ChevronLeft, ChevronRight, X, ArrowRight } from "lucide-react";
import type { CheckpointData, DailyLog, DailyQuestLink } from "../types";

type Quadrant = "do" | "schedule" | "delegate" | "eliminate";

interface QuadrantMeta {
  key: Quadrant;
  label: string;
  desc: string;
  color: string;
}

const QUADRANTS: QuadrantMeta[] = [
  { key: "do", label: "Do", desc: "Urgent + Important", color: "#dc2626" },
  { key: "schedule", label: "Schedule", desc: "Important, not urgent", color: "#2563eb" },
  { key: "delegate", label: "Delegate", desc: "Urgent, not important", color: "#d97706" },
  { key: "eliminate", label: "Eliminate", desc: "Neither", color: "#64748b" },
];

function getCheckpoint(tasks: unknown): CheckpointData | null {
  if (!tasks) return null;
  if (Array.isArray(tasks)) {
    const t = tasks[0];
    if (!t) return null;
    return {
      mastery_criteria: t.description ?? "",
      exercises: [],
      notes: t.notes ?? "",
      completed: t.completed ?? false,
    };
  }
  return tasks as CheckpointData;
}

function formatDate(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

function displayDate(date: Date): string {
  return date.toLocaleDateString("en-US", {
    weekday: "long",
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

function isToday(date: Date): boolean {
  const now = new Date();
  return formatDate(date) === formatDate(now);
}

function isPast(date: Date): boolean {
  const now = new Date();
  return formatDate(date) < formatDate(now);
}

export default function DailyPage() {
  const [currentDate, setCurrentDate] = useState<Date>(new Date());
  const [log, setLog] = useState<DailyLog | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [toggling, setToggling] = useState<Set<string>>(new Set());
  const [addingTo, setAddingTo] = useState<Quadrant | null>(null);
  const [addText, setAddText] = useState("");
  const [notes, setNotes] = useState("");
  const [moveOpen, setMoveOpen] = useState<string | null>(null);
  const addInputRef = useRef<HTMLInputElement>(null);

  const readOnly = isPast(currentDate);
  const dateStr = formatDate(currentDate);

  const loadLog = useCallback(async () => {
    try {
      setLoading(true);
      const data = await invoke<DailyLog>("get_daily_log", { date: dateStr });
      setLog(data);
      setNotes(data.notes ?? "");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [dateStr]);

  useEffect(() => {
    loadLog();
    setAddingTo(null);
    setAddText("");
    setMoveOpen(null);
  }, [loadLog]);

  useEffect(() => {
    if (addingTo && addInputRef.current) {
      addInputRef.current.focus();
    }
  }, [addingTo]);

  function shiftDate(delta: number) {
    setCurrentDate((prev) => {
      const next = new Date(prev);
      next.setDate(next.getDate() + delta);
      return next;
    });
  }

  function goToToday() {
    setCurrentDate(new Date());
  }

  async function toggleCompletion(link: DailyQuestLink) {
    if (!link.nodeId || readOnly) return;
    setToggling((prev) => new Set(prev).add(link.id));
    try {
      const nodeData = await invoke<{ tasks: unknown; treeId: string; projectId: string }>(
        "get_tree_node_data",
        { nodeId: link.nodeId }
      );
      const cp = getCheckpoint(nodeData.tasks);
      if (!cp) return;
      const newCompleted = !cp.completed;
      const newTasks = { ...cp, completed: newCompleted };
      const newProgress = newCompleted ? 100 : 0;

      await invoke("update_tree_node", {
        nodeId: link.nodeId,
        title: null,
        description: null,
        progress: newProgress,
        tasks: newTasks,
        resources: null,
        position: null,
      });

      await invoke("recalculate_tree_progress", { treeId: nodeData.treeId });
      await invoke("update_project_progress", { projectId: nodeData.projectId });
      invoke("sync_skills_from_trees")
        .then(() => invoke("recalculate_skill_levels"))
        .catch(console.warn);

      await loadLog();
    } catch (e) {
      console.error("Failed to toggle completion:", e);
    } finally {
      setToggling((prev) => {
        const next = new Set(prev);
        next.delete(link.id);
        return next;
      });
    }
  }

  async function handleAddTask(quadrant: Quadrant) {
    const text = addText.trim();
    if (!text) {
      setAddingTo(null);
      setAddText("");
      return;
    }
    try {
      await invoke("add_free_task_to_day", { date: dateStr, text, quadrant });
      setAddText("");
      setAddingTo(null);
      await loadLog();
    } catch (e) {
      console.error("Failed to add task:", e);
    }
  }

  async function handleRemove(linkId: string) {
    if (readOnly) return;
    try {
      await invoke("remove_from_day", { linkId });
      await loadLog();
    } catch (e) {
      console.error("Failed to remove:", e);
    }
  }

  async function handleMove(linkId: string, quadrant: Quadrant) {
    if (readOnly) return;
    try {
      await invoke("move_to_quadrant", { linkId, quadrant });
      setMoveOpen(null);
      await loadLog();
    } catch (e) {
      console.error("Failed to move:", e);
    }
  }

  async function saveNotes() {
    if (readOnly) return;
    try {
      await invoke("upsert_daily_notes", { date: dateStr, notes });
    } catch (e) {
      console.error("Failed to save notes:", e);
    }
  }

  function linksForQuadrant(q: Quadrant): DailyQuestLink[] {
    if (!log) return [];
    return log.links
      .filter((l) => l.quadrant === q)
      .sort((a, b) => a.sortOrder - b.sortOrder);
  }

  if (loading) {
    return <div className="p-6 text-slate-400 text-sm">Loading daily log...</div>;
  }

  if (error) {
    return <div className="p-6 text-red-400 text-sm">Failed to load daily log: {error}</div>;
  }

  return (
    <div className="p-6 flex flex-col gap-4 h-full min-h-0">
      {/* Header with date nav */}
      <div className="flex items-center justify-between flex-shrink-0">
        <div>
          <h1 className="text-xl font-semibold text-slate-100">Daily Matrix</h1>
          {readOnly && (
            <p className="text-xs text-amber-500 mt-0.5">Read-only — past day</p>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => shiftDate(-1)}
            className="p-1.5 rounded-md hover:bg-slate-800 text-slate-400 hover:text-slate-200 transition-colors"
          >
            <ChevronLeft className="w-4 h-4" />
          </button>
          <button
            onClick={goToToday}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-slate-900 border border-slate-700 text-sm text-slate-200 hover:border-slate-600 transition-colors"
          >
            <CalendarDays className="w-3.5 h-3.5" />
            {isToday(currentDate) ? "Today" : displayDate(currentDate)}
          </button>
          <button
            onClick={() => shiftDate(1)}
            className="p-1.5 rounded-md hover:bg-slate-800 text-slate-400 hover:text-slate-200 transition-colors"
          >
            <ChevronRight className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* 2x2 Quadrant Grid */}
      <div className="grid grid-cols-2 grid-rows-2 gap-3 flex-1 min-h-0">
        {QUADRANTS.map((qm) => {
          const items = linksForQuadrant(qm.key);
          return (
            <div
              key={qm.key}
              className="bg-slate-900 border border-slate-800 rounded-lg flex flex-col min-h-0 overflow-hidden"
            >
              {/* Quadrant header */}
              <div
                className="px-3 py-2 flex items-center justify-between flex-shrink-0"
                style={{ borderBottom: `2px solid ${qm.color}` }}
              >
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold" style={{ color: qm.color }}>
                    {qm.label}
                  </span>
                  <span className="text-xs text-slate-500">{qm.desc}</span>
                </div>
                <span
                  className="text-xs font-medium px-1.5 py-0.5 rounded"
                  style={{ backgroundColor: qm.color + "20", color: qm.color }}
                >
                  {items.length}
                </span>
              </div>

              {/* Scrollable items */}
              <div className="flex-1 overflow-y-auto p-2 flex flex-col gap-1">
                {items.map((link) => {
                  const isLinked = !!link.nodeId;
                  const isComplete = isLinked && link.nodeProgress === 100;
                  const isLocked = link.nodeIsLocked === true;
                  const isTogglingThis = toggling.has(link.id);
                  const title = isLinked ? link.nodeTitle ?? "Untitled" : link.freeText ?? "";

                  return (
                    <div
                      key={link.id}
                      className={`group relative flex items-center gap-2 px-2 py-1.5 rounded-md hover:bg-slate-800/50 transition-colors ${
                        isComplete ? "opacity-50" : ""
                      }`}
                    >
                      {/* Checkbox or marker */}
                      {isLinked ? (
                        <button
                          onClick={() => toggleCompletion(link)}
                          disabled={isTogglingThis || readOnly || isLocked}
                          className={`flex-shrink-0 w-4 h-4 rounded border-2 flex items-center justify-center transition-colors ${
                            isComplete
                              ? "bg-emerald-600 border-emerald-600"
                              : isLocked
                              ? "border-slate-700 cursor-not-allowed"
                              : "border-slate-600 hover:border-emerald-500 cursor-pointer"
                          } ${isTogglingThis ? "opacity-50 cursor-wait" : ""}`}
                        >
                          {isComplete && (
                            <svg className="w-2.5 h-2.5 text-white" fill="none" viewBox="0 0 12 12">
                              <path d="M2 6l3 3 5-5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
                            </svg>
                          )}
                        </button>
                      ) : (
                        <span className="flex-shrink-0 w-4 h-4 flex items-center justify-center">
                          <span className="w-2.5 h-2.5 rounded-sm bg-slate-600" />
                        </span>
                      )}

                      {/* Title and progress */}
                      <div className="flex-1 min-w-0">
                        <span
                          className={`text-sm leading-tight ${
                            isComplete ? "line-through text-slate-500" : "text-slate-200"
                          }`}
                        >
                          {title}
                        </span>
                        {isLocked && (
                          <span className="ml-1.5 text-[10px] text-amber-500 font-medium uppercase tracking-wider">
                            locked
                          </span>
                        )}
                        {isLinked && !isComplete && link.nodeProgress != null && link.nodeProgress > 0 && (
                          <div className="mt-1 w-full h-1 bg-slate-800 rounded-full overflow-hidden">
                            <div
                              className="h-full bg-emerald-500 rounded-full"
                              style={{ width: `${link.nodeProgress}%` }}
                            />
                          </div>
                        )}
                      </div>

                      {/* Hover actions */}
                      {!readOnly && (
                        <div className="hidden group-hover:flex items-center gap-0.5 flex-shrink-0">
                          {/* Move dropdown */}
                          <div className="relative">
                            <button
                              onClick={() => setMoveOpen(moveOpen === link.id ? null : link.id)}
                              className="p-0.5 rounded hover:bg-slate-700 text-slate-500 hover:text-slate-300 transition-colors"
                              title="Move to quadrant"
                            >
                              <ArrowRight className="w-3.5 h-3.5" />
                            </button>
                            {moveOpen === link.id && (
                              <div className="absolute right-0 top-full mt-1 z-20 bg-slate-800 border border-slate-700 rounded-md shadow-lg py-1 min-w-[120px]">
                                {QUADRANTS.filter((q) => q.key !== qm.key).map((q) => (
                                  <button
                                    key={q.key}
                                    onMouseDown={(e) => e.preventDefault()}
                                    onClick={() => handleMove(link.id, q.key)}
                                    className="w-full text-left px-3 py-1 text-xs hover:bg-slate-700 transition-colors"
                                    style={{ color: q.color }}
                                  >
                                    {q.label}
                                  </button>
                                ))}
                              </div>
                            )}
                          </div>
                          {/* Remove */}
                          <button
                            onClick={() => handleRemove(link.id)}
                            className="p-0.5 rounded hover:bg-slate-700 text-slate-500 hover:text-red-400 transition-colors"
                            title="Remove from day"
                          >
                            <X className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      )}
                    </div>
                  );
                })}

                {items.length === 0 && (
                  <div className="text-xs text-slate-600 text-center py-3">No tasks</div>
                )}
              </div>

              {/* Add task input */}
              {!readOnly && (
                <div className="flex-shrink-0 border-t border-slate-800">
                  {addingTo === qm.key ? (
                    <input
                      ref={addInputRef}
                      type="text"
                      value={addText}
                      onChange={(e) => setAddText(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleAddTask(qm.key);
                        if (e.key === "Escape") {
                          setAddingTo(null);
                          setAddText("");
                        }
                      }}
                      onBlur={() => {
                        // Delay to allow click events to register
                        setTimeout(() => {
                          setAddingTo(null);
                          setAddText("");
                        }, 150);
                      }}
                      placeholder="Type task and press Enter..."
                      className="w-full px-3 py-2 bg-transparent text-sm text-slate-200 placeholder-slate-600 outline-none"
                    />
                  ) : (
                    <button
                      onClick={() => {
                        setAddingTo(qm.key);
                        setAddText("");
                      }}
                      className="w-full px-3 py-2 text-xs text-slate-500 hover:text-slate-300 hover:bg-slate-800/50 transition-colors text-left"
                    >
                      + add task
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {/* Notes section */}
      <div className="flex-shrink-0">
        <label className="text-xs text-slate-500 font-medium mb-1 block">Daily Notes</label>
        <textarea
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          onBlur={saveNotes}
          disabled={readOnly}
          placeholder={readOnly ? "" : "Jot down thoughts, reflections, blockers..."}
          className="w-full h-20 bg-slate-900 border border-slate-800 rounded-lg px-3 py-2 text-sm text-slate-200 placeholder-slate-600 resize-none outline-none focus:border-slate-600 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
        />
      </div>
    </div>
  );
}

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, NotebookPen } from "lucide-react";
import type { QuestNode } from "../types";

const DIFFICULTY_STYLES: Record<string, string> = {
  easy: "bg-emerald-900/60 text-emerald-300 border-emerald-700",
  medium: "bg-yellow-900/60 text-yellow-300 border-yellow-700",
  hard: "bg-red-900/60 text-red-300 border-red-700",
};

export default function QuestsPage() {
  const [quests, setQuests] = useState<QuestNode[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filterProject, setFilterProject] = useState("all");
  const [filterStatus, setFilterStatus] = useState("all");
  const [filterDifficulty, setFilterDifficulty] = useState("all");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [toggling, setToggling] = useState<Set<string>>(new Set());

  useEffect(() => {
    loadQuests();
  }, []);

  async function loadQuests() {
    try {
      const data = await invoke<QuestNode[]>("get_all_quests");
      setQuests(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function toggleQuest(quest: QuestNode) {
    const task = quest.tasks[0] ?? null;
    if (!task) return;

    const newCompleted = !task.completed;
    const newTasks = [{ ...task, completed: newCompleted }];
    const newProgress = newCompleted ? 100 : 0;

    setToggling((prev) => new Set(prev).add(quest.id));

    try {
      await invoke("update_tree_node", {
        nodeId: quest.id,
        title: null,
        description: null,
        progress: newProgress,
        tasks: newTasks,
        resources: null,
        position: null,
      });

      // Propagate progress up through the tree, then update the project card
      await invoke("recalculate_tree_progress", { treeId: quest.treeId });
      await invoke("update_project_progress", { projectId: quest.projectId });

      setQuests((prev) =>
        prev.map((q) =>
          q.id === quest.id
            ? { ...q, tasks: newTasks, progress: newProgress }
            : q
        )
      );
    } catch (e) {
      console.error("Failed to toggle quest:", e);
    } finally {
      setToggling((prev) => {
        const next = new Set(prev);
        next.delete(quest.id);
        return next;
      });
    }
  }

  function toggleExpand(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // Unique project list for filter dropdown
  const projectOptions = useMemo(() => {
    const seen = new Map<string, string>();
    quests.forEach((q) => seen.set(q.projectId, q.projectName));
    return Array.from(seen.entries());
  }, [quests]);

  // Filtered view
  const filtered = useMemo(() => {
    return quests.filter((q) => {
      const task = q.tasks[0] ?? null;
      if (filterProject !== "all" && q.projectId !== filterProject) return false;
      if (filterStatus === "complete" && q.progress < 100) return false;
      if (filterStatus === "incomplete" && q.progress >= 100) return false;
      if (filterDifficulty !== "all" && task?.difficulty !== filterDifficulty) return false;
      return true;
    });
  }, [quests, filterProject, filterStatus, filterDifficulty]);

  // Stats
  const stats = useMemo(() => {
    const total = quests.length;
    const done = quests.filter((q) => q.progress >= 100).length;
    const hoursRemaining = quests.reduce((sum, q) => {
      if (q.progress >= 100) return sum;
      const task = q.tasks[0] ?? null;
      return sum + (task?.estimated_hours ?? 0);
    }, 0);
    return { total, done, hoursRemaining };
  }, [quests]);

  if (loading) {
    return (
      <div className="p-6 text-slate-400 text-sm">Loading quests...</div>
    );
  }

  if (error) {
    return (
      <div className="p-6 text-red-400 text-sm">
        Failed to load quests: {error}
      </div>
    );
  }

  return (
    <div className="p-6 max-w-4xl mx-auto flex flex-col gap-6">
      {/* Header */}
      <div>
        <h1 className="text-xl font-semibold text-slate-100">Quests</h1>
        <p className="text-sm text-slate-400 mt-1">
          All learning tasks across your projects.
        </p>
      </div>

      {/* Stats banner */}
      {stats.total > 0 && (
        <div className="bg-slate-900 border border-slate-800 rounded-lg p-4 flex flex-col gap-3">
          <div className="flex items-center justify-between text-sm">
            <span className="text-slate-300 font-medium">
              {stats.done} / {stats.total} quests complete
            </span>
            <span className="text-slate-400">
              ~{stats.hoursRemaining.toFixed(1)}h remaining
            </span>
          </div>
          <div className="w-full h-2 bg-slate-800 rounded-full overflow-hidden">
            <div
              className="h-full bg-emerald-500 rounded-full transition-all duration-300"
              style={{
                width: `${stats.total > 0 ? (stats.done / stats.total) * 100 : 0}%`,
              }}
            />
          </div>
        </div>
      )}

      {/* Filters */}
      <div className="flex flex-wrap gap-3">
        <select
          value={filterProject}
          onChange={(e) => setFilterProject(e.target.value)}
          className="bg-slate-900 border border-slate-700 text-slate-300 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-emerald-600"
        >
          <option value="all">All Projects</option>
          {projectOptions.map(([id, name]) => (
            <option key={id} value={id}>
              {name}
            </option>
          ))}
        </select>

        <select
          value={filterStatus}
          onChange={(e) => setFilterStatus(e.target.value)}
          className="bg-slate-900 border border-slate-700 text-slate-300 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-emerald-600"
        >
          <option value="all">All Status</option>
          <option value="incomplete">Incomplete</option>
          <option value="complete">Complete</option>
        </select>

        <select
          value={filterDifficulty}
          onChange={(e) => setFilterDifficulty(e.target.value)}
          className="bg-slate-900 border border-slate-700 text-slate-300 text-sm rounded-md px-3 py-1.5 focus:outline-none focus:border-emerald-600"
        >
          <option value="all">All Difficulties</option>
          <option value="easy">Easy</option>
          <option value="medium">Medium</option>
          <option value="hard">Hard</option>
        </select>

        {filtered.length !== quests.length && (
          <span className="text-slate-500 text-sm self-center">
            {filtered.length} shown
          </span>
        )}
      </div>

      {/* Quest list */}
      {filtered.length === 0 ? (
        <div className="text-slate-500 text-sm">
          {quests.length === 0
            ? "No quests found. Generate an AI tree to create quests."
            : "No quests match the current filters."}
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {filtered.map((quest) => {
            const task = quest.tasks[0] ?? null;
            const isComplete = quest.progress >= 100;
            const isExpanded = expanded.has(quest.id);
            const isToggling = toggling.has(quest.id);

            return (
              <div
                key={quest.id}
                className={`bg-slate-900 border rounded-lg p-4 flex gap-3 transition-colors ${
                  isComplete ? "border-slate-700 opacity-70" : "border-slate-800"
                }`}
              >
                {/* Checkbox */}
                <button
                  onClick={() => toggleQuest(quest)}
                  disabled={isToggling}
                  className={`mt-0.5 flex-shrink-0 w-5 h-5 rounded border-2 flex items-center justify-center transition-colors ${
                    isComplete
                      ? "bg-emerald-600 border-emerald-600"
                      : "border-slate-600 hover:border-emerald-500"
                  } ${isToggling ? "opacity-50 cursor-wait" : "cursor-pointer"}`}
                  aria-label={isComplete ? "Mark incomplete" : "Mark complete"}
                >
                  {isComplete && (
                    <svg className="w-3 h-3 text-white" fill="none" viewBox="0 0 12 12">
                      <path
                        d="M2 6l3 3 5-5"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      />
                    </svg>
                  )}
                </button>

                {/* Content */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <span
                        className={`text-sm font-medium leading-snug ${
                          isComplete
                            ? "line-through text-slate-500"
                            : "text-slate-100"
                        }`}
                      >
                        {quest.title}
                      </span>
                      {task?.notes?.trim() && (
                        <span title="Has notes"><NotebookPen className="w-3 h-3 text-slate-500 flex-shrink-0" /></span>
                      )}
                    </div>

                    <div className="flex items-center gap-2 flex-shrink-0">
                      {task?.difficulty && (
                        <span
                          className={`text-xs px-2 py-0.5 rounded border ${
                            DIFFICULTY_STYLES[task.difficulty] ??
                            DIFFICULTY_STYLES.medium
                          }`}
                        >
                          {task.difficulty}
                        </span>
                      )}
                      {task?.estimated_hours != null && (
                        <span className="text-xs text-slate-500">
                          {task.estimated_hours}h
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Breadcrumb */}
                  <div className="text-xs text-slate-500 mt-1">
                    {quest.projectName} › {quest.treeName}
                  </div>

                  {/* Expandable description */}
                  {quest.description && (
                    <div className="mt-1.5">
                      {isExpanded && (
                        <p className="text-xs text-slate-400 leading-relaxed mb-1">
                          {quest.description}
                        </p>
                      )}
                      <button
                        onClick={() => toggleExpand(quest.id)}
                        className="text-slate-600 hover:text-slate-400 transition-colors"
                        aria-label={isExpanded ? "Collapse" : "Expand"}
                      >
                        <ChevronDown
                          className={`w-3.5 h-3.5 transition-transform duration-150 ${isExpanded ? "rotate-180" : ""}`}
                        />
                      </button>
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

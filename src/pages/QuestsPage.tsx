import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, NotebookPen } from "lucide-react";
import type { QuestNode, CheckpointData } from "../types";

/** Extract checkpoint data from tasks field, handling both legacy array and new object shape */
function getCheckpoint(quest: QuestNode): CheckpointData | null {
  if (!quest.tasks) return null;
  if (Array.isArray(quest.tasks)) {
    const t = quest.tasks[0];
    if (!t) return null;
    return {
      mastery_criteria: t.description ?? '',
      exercises: [],
      notes: t.notes ?? '',
      completed: t.completed ?? false,
    };
  }
  return quest.tasks as CheckpointData;
}

export default function QuestsPage() {
  const [quests, setQuests] = useState<QuestNode[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filterProject, setFilterProject] = useState("all");
  const [filterStatus, setFilterStatus] = useState("all");
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
    const cp = getCheckpoint(quest);
    if (!cp) return;

    const newCompleted = !cp.completed;
    const newTasks = { ...cp, completed: newCompleted };
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

      await invoke("recalculate_tree_progress", { treeId: quest.treeId });
      await invoke("update_project_progress", { projectId: quest.projectId });
      invoke("sync_skills_from_trees").then(() => invoke("recalculate_skill_levels")).catch(console.warn);

      setQuests((prev) =>
        prev.map((q) =>
          q.id === quest.id
            ? { ...q, tasks: newTasks, progress: newProgress }
            : q
        )
      );
    } catch (e) {
      console.error("Failed to toggle checkpoint:", e);
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

  const projectOptions = useMemo(() => {
    const seen = new Map<string, string>();
    quests.forEach((q) => seen.set(q.projectId, q.projectName));
    return Array.from(seen.entries());
  }, [quests]);

  const filtered = useMemo(() => {
    return quests.filter((q) => {
      if (filterProject !== "all" && q.projectId !== filterProject) return false;
      if (filterStatus === "complete" && q.progress < 100) return false;
      if (filterStatus === "incomplete" && q.progress >= 100) return false;
      return true;
    });
  }, [quests, filterProject, filterStatus]);

  const stats = useMemo(() => {
    const total = quests.length;
    const done = quests.filter((q) => q.progress >= 100).length;
    return { total, done };
  }, [quests]);

  if (loading) {
    return (
      <div className="p-6 text-slate-400 text-sm">Loading checkpoints...</div>
    );
  }

  if (error) {
    return (
      <div className="p-6 text-red-400 text-sm">
        Failed to load checkpoints: {error}
      </div>
    );
  }

  return (
    <div className="p-6 max-w-4xl mx-auto flex flex-col gap-6">
      {/* Header */}
      <div>
        <h1 className="text-xl font-semibold text-slate-100">Checkpoints</h1>
        <p className="text-sm text-slate-400 mt-1">
          All concept checkpoints across your projects.
        </p>
      </div>

      {/* Stats banner */}
      {stats.total > 0 && (
        <div className="bg-slate-900 border border-slate-800 rounded-lg p-4 flex flex-col gap-3">
          <div className="flex items-center justify-between text-sm">
            <span className="text-slate-300 font-medium">
              {stats.done} / {stats.total} checkpoints reached
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
          <option value="incomplete">Not Reached</option>
          <option value="complete">Reached</option>
        </select>

        {filtered.length !== quests.length && (
          <span className="text-slate-500 text-sm self-center">
            {filtered.length} shown
          </span>
        )}
      </div>

      {/* Checkpoint list */}
      {filtered.length === 0 ? (
        <div className="text-slate-500 text-sm">
          {quests.length === 0
            ? "No checkpoints found. Generate an AI tree to create checkpoints."
            : "No checkpoints match the current filters."}
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {filtered.map((quest) => {
            const cp = getCheckpoint(quest);
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
                  aria-label={isComplete ? "Mark not reached" : "Mark reached"}
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
                      {cp?.notes?.trim() && (
                        <span title="Has notes"><NotebookPen className="w-3 h-3 text-slate-500 flex-shrink-0" /></span>
                      )}
                    </div>
                  </div>

                  {/* Breadcrumb */}
                  <div className="text-xs text-slate-500 mt-1">
                    {quest.projectName} › {quest.treeName}
                  </div>

                  {/* Expandable mastery criteria */}
                  {(cp?.mastery_criteria || quest.description) && (
                    <div className="mt-1.5">
                      {isExpanded && (
                        <p className="text-xs text-slate-400 leading-relaxed mb-1">
                          {cp?.mastery_criteria || quest.description}
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

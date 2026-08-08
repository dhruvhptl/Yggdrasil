import { useState } from "react";
import { ChevronRight, ChevronDown } from "lucide-react";
import { StatusBadge, type ToolStatus } from "./StatusBadge";

const TOOL_ICON: Record<string, string> = {
  search_mimir: "🔍", smart_search: "🌐", smart_fetch: "📄",
  read_tree: "📖", get_facts: "🧠", set_fact: "🧠",
  query_graph: "🕸️", path_between: "🧭", explain_node: "💡", scan_project: "🗂️",
};

export interface ToolCallBlockProps {
  toolName: string;
  status: ToolStatus;
  input?: unknown;
  output?: string;
  durationMs?: number;
  defaultExpanded?: boolean;
}

export function ToolCallBlock({ toolName, status, input, output, durationMs, defaultExpanded = false }: ToolCallBlockProps) {
  const [open, setOpen] = useState(defaultExpanded);
  const isError = status === "error";
  return (
    <div className={`rounded-md border text-sm ${isError ? "border-red-500/50 bg-red-500/5" : "border-white/10 bg-white/5"}`}>
      <button onClick={() => setOpen(o => !o)} className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left">
        {open ? <ChevronDown className="w-3.5 h-3.5 opacity-60" /> : <ChevronRight className="w-3.5 h-3.5 opacity-60" />}
        <span>{TOOL_ICON[toolName] ?? "🛠️"}</span>
        <span className="font-mono text-xs">{toolName}</span>
        <StatusBadge status={status} />
        {durationMs != null && <span className="ml-auto text-xs opacity-50">{(durationMs / 1000).toFixed(1)}s</span>}
      </button>
      {open && (
        <div className="px-3 pb-2 space-y-1.5">
          {input != null && (
            <pre className="text-xs opacity-70 whitespace-pre-wrap break-words">{JSON.stringify(input, null, 2)}</pre>
          )}
          {output && (
            <pre className={`text-xs whitespace-pre-wrap break-words ${isError ? "text-red-300" : "opacity-80"}`}>{output}</pre>
          )}
        </div>
      )}
    </div>
  );
}

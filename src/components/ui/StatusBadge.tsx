import { Loader2, Check, X } from "lucide-react";

export type ToolStatus = "running" | "success" | "error";

export function StatusBadge({ status }: { status: ToolStatus }) {
  if (status === "running") return <Loader2 className="w-3.5 h-3.5 animate-spin text-blue-400" />;
  if (status === "success") return <Check className="w-3.5 h-3.5 text-green-400" />;
  return <X className="w-3.5 h-3.5 text-red-400" />;
}

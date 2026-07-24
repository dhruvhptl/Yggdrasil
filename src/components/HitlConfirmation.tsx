// src/components/HitlConfirmation.tsx
import { invoke } from "@tauri-apps/api/core";

export interface ActionProposal {
  actionType: string;
  summary: string;
  params: Record<string, unknown>;
}

interface Props {
  proposal: ActionProposal;
  treeId: string | null;
  onResolved: (resultMessage: string) => void;
}

export function HitlConfirmation({ proposal, treeId, onResolved }: Props) {
  const approve = async () => {
    try {
      const result = await invoke<string>("execute_destructive_action_cmd", {
        actionType: proposal.actionType,
        params: proposal.params,
        treeId: treeId,
      });
      onResolved(result);
    } catch (e) {
      onResolved(`Action failed: ${String(e)}`);
    }
  };
  const reject = () => onResolved("Okay, I won't do that.");

  return (
    <div
      style={{
        marginTop: 8,
        padding: 12,
        borderRadius: 8,
        background: "rgba(30, 41, 59, 0.6)",
        border: "1px solid #334155",
      }}
    >
      <div style={{ marginBottom: 10, fontSize: 13, color: "#e2e8f0", lineHeight: 1.5 }}>
        <strong style={{ color: "#f1f5f9" }}>Mimir wants to:</strong> {proposal.summary}
      </div>
      <div style={{ display: "flex", gap: 8 }}>
        <button
          onClick={approve}
          style={{
            padding: "6px 12px",
            borderRadius: 6,
            border: "none",
            background: "#059669",
            color: "#ffffff",
            fontSize: 12,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          Approve
        </button>
        <button
          onClick={reject}
          style={{
            padding: "6px 12px",
            borderRadius: 6,
            border: "1px solid #334155",
            background: "transparent",
            color: "#94a3b8",
            fontSize: 12,
            fontWeight: 600,
            cursor: "pointer",
          }}
        >
          Reject
        </button>
      </div>
    </div>
  );
}

export interface Project {
  id: string;
  name: string;
  description: string;
  disciplineIds: string[];
  skillIds: string[];
  status: string;
  createdAt: string;
  progress: number;
}

// Quest tracking types

export interface QuestTask {
  id: string;
  title: string;
  description: string;
  estimated_hours: number;
  difficulty: "easy" | "medium" | "hard";
  completed: boolean;
  notes?: string;
}

export interface QuestNode {
  id: string;
  treeId: string;
  projectId: string;
  projectName: string;
  treeName: string;
  title: string;
  description: string;
  progress: number;
  tasks: QuestTask[];
  orderIndex: number;
}

// Add to src/types.ts at the bottom

// Brain-generated tree structure (Phases → Skills → Quests)
export interface GeneratedTree {
  project_id: string;
  phases: Phase[];
}

export interface Phase {
  id: string;
  name: string;
  description: string;
  order: number;
  skills: BrainSkill[];
}

export interface BrainSkill {
  id: string;
  name: string;
  description: string;
  order: number;
  quests: BrainQuest[];
}

export interface BrainQuest {
  id: string;
  title: string;
  description: string;
  estimated_hours: number;
  difficulty: "easy" | "medium" | "hard";
  order: number;
}

// Mimir library resource (from Rust mimir.rs / sidecar)
export interface MimirResource {
  id: string;
  title: string;
  url: string | null;
  resourceType: string;  // Rust renames `type` → `resource_type` → camelCase `resourceType`
  status: string;
  userNotes: string | null;
  createdAt: string;
  relevanceScore?: number | null;
}

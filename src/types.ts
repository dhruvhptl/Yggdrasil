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

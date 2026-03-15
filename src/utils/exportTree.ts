// src/utils/exportTree.ts
import { invoke } from '@tauri-apps/api/core';
import JSZip from 'jszip';

interface ExportResource {
  title: string;
  url: string | null;
  resourceType: string | null;
}

interface ExportQuest {
  title: string;
  masteryCriteria: string;
  exercises: string[];
  resources: ExportResource[];
}

interface ExportSkill {
  name: string;
  quests: ExportQuest[];
}

interface ExportPhase {
  name: string;
  skills: ExportSkill[];
}

interface ExportProject {
  name: string;
  description: string;
}

interface ExportTree {
  project: ExportProject;
  phases: ExportPhase[];
}

/** Build the full nested JSON structure */
function buildJson(tree: ExportTree) {
  return {
    project: tree.project,
    phases: tree.phases.map(p => ({
      name: p.name,
      skills: p.skills.map(s => ({
        name: s.name,
        checkpoints: s.quests.map(q => ({
          title: q.title,
          mastery_criteria: q.masteryCriteria,
          exercises: q.exercises,
          resources: q.resources.map(r => ({
            title: r.title,
            url: r.url,
            type: r.resourceType,
          })),
        })),
      })),
    })),
  };
}

/** Build JSONL: one tree-level record + one per quest */
function buildJsonl(tree: ExportTree): string {
  const lines: string[] = [];

  // Tree-level record
  const fullJson = buildJson(tree);
  lines.push(JSON.stringify({
    type: 'tree',
    project: tree.project.name,
    output: fullJson.phases,
  }));

  // Per-checkpoint records
  for (const phase of tree.phases) {
    for (const skill of phase.skills) {
      for (const quest of skill.quests) {
        lines.push(JSON.stringify({
          type: 'checkpoint',
          input: `${quest.title}: ${quest.masteryCriteria}`,
          output: quest.resources.map(r => ({
            title: r.title,
            url: r.url,
            type: r.resourceType,
          })),
        }));
      }
    }
  }

  return lines.join('\n') + '\n';
}

/** Trigger browser download of a Blob */
function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/** Export a project's tree as a .zip containing .json + .jsonl */
export async function exportTreeAsZip(projectId: string): Promise<void> {
  const tree = await invoke<ExportTree>('export_tree', { projectId });

  if (tree.phases.length === 0) {
    throw new Error('No tree found for this project');
  }

  const slug = tree.project.name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');

  const jsonContent = JSON.stringify(buildJson(tree), null, 2);
  const jsonlContent = buildJsonl(tree);

  const zip = new JSZip();
  zip.file(`${slug}.json`, jsonContent);
  zip.file(`${slug}.jsonl`, jsonlContent);

  const blob = await zip.generateAsync({ type: 'blob' });
  downloadBlob(blob, `${slug}-export.zip`);
}

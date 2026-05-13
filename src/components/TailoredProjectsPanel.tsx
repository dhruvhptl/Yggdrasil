import { useState, useEffect } from 'react';
import { X, ExternalLink } from 'lucide-react';

interface TailoredProject {
  projectName: string;
  projectDescription?: string;
  yggdrasilProjectId?: string;
  matchedSkills: string[];
  missingRequiredSkills: string[];
  matchScore: number;
  talkingPoints: string[];
}

interface TailoredProjects {
  jobId: string;
  company: string;
  position: string;
  requiredSkillsCount: number;
  topProjects: TailoredProject[];
}

interface TailoredProjectsPanelProps {
  job: { id: string; company: string; position: string };
  tailoredData: TailoredProjects;
  loading?: boolean;
  onClose: () => void;
}

export function TailoredProjectsPanel({
  job,
  tailoredData,
  loading = false,
  onClose,
}: TailoredProjectsPanelProps) {
  const [copyNotification, setCopyNotification] = useState(false);

  useEffect(() => {
    if (!copyNotification) return;

    const timeoutId = setTimeout(() => {
      setCopyNotification(false);
    }, 2000);

    return () => clearTimeout(timeoutId);
  }, [copyNotification]);

  async function handleCopyToClipboard() {
    const text = tailoredData.topProjects
      .map((p) => `• ${p.projectName}: ${p.projectDescription || 'N/A'} (${p.matchedSkills.join(', ')}); ${p.talkingPoints.join('; ')}`)
      .join('\n');

    try {
      await navigator.clipboard.writeText(text);
      setCopyNotification(true);
    } catch (error) {
      console.error('Failed to copy to clipboard:', error);
    }
  }

  function handleOpenResume() {
    window.location.href = '/resume';
  }

  return (
    <div className="fixed right-0 top-0 h-full w-96 bg-slate-900 border-l border-slate-700 shadow-2xl z-40 flex flex-col overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b border-slate-700">
        <div>
          <p className="text-white font-semibold text-sm">{job.company}</p>
          <p className="text-slate-400 text-xs">{job.position}</p>
        </div>
        <button onClick={onClose} className="text-slate-400 hover:text-white">
          <X className="w-4 h-4" />
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4">
        {loading ? (
          <p className="text-slate-400 text-sm">Loading...</p>
        ) : tailoredData.topProjects.length === 0 ? (
          <p className="text-slate-400 text-sm">No resume projects matched.</p>
        ) : (
          <div className="space-y-3">
            {tailoredData.topProjects.map((project, idx) => (
              <div key={idx} className="bg-slate-800 border border-slate-700 rounded-lg p-3">
                {/* Project Name + Yggdrasil Link */}
                <div className="flex items-center gap-2 mb-2">
                  <p className="text-white font-medium text-sm">{project.projectName}</p>
                  {project.yggdrasilProjectId && (
                    <a
                      href={`/projects/${project.yggdrasilProjectId}`}
                      className="text-blue-400 hover:text-blue-300"
                      target="_blank"
                      rel="noopener noreferrer"
                    >
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  )}
                </div>

                {/* Match Score Bar */}
                <div className="mb-2">
                  <div className="flex items-center justify-between mb-1">
                    <span className="text-xs text-slate-400">
                      {Math.round(project.matchScore * 100)}% match ({project.matchedSkills.length}/{tailoredData.requiredSkillsCount} skills)
                    </span>
                  </div>
                  <div className="w-full bg-slate-700 rounded-full h-2">
                    <div
                      className="h-2 rounded-full transition-all"
                      style={{
                        width: `${project.matchScore * 100}%`,
                        backgroundColor: project.matchScore < 0.33 ? '#ef4444' : project.matchScore < 0.66 ? '#f59e0b' : '#10b981',
                      }}
                    />
                  </div>
                </div>

                {/* Matched Skills */}
                {project.matchedSkills.length > 0 && (
                  <div className="mb-2">
                    <p className="text-xs text-slate-400 mb-1">Matched skills</p>
                    <div className="flex flex-wrap gap-1">
                      {project.matchedSkills.map((skill) => (
                        <span key={`matched-skill-${idx}-${skill}`} className="bg-emerald-900 text-emerald-100 text-xs px-2 py-0.5 rounded-full">
                          {skill}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Missing Skills */}
                {project.missingRequiredSkills.length > 0 && (
                  <div className="mb-2">
                    <p className="text-xs text-slate-400 mb-1">Missing skills</p>
                    <div className="flex flex-wrap gap-1">
                      {project.missingRequiredSkills.map((skill) => (
                        <span key={`missing-skill-${idx}-${skill}`} className="bg-amber-900 text-amber-100 text-xs px-2 py-0.5 rounded-full">
                          {skill}
                        </span>
                      ))}
                    </div>
                  </div>
                )}

                {/* Talking Points */}
                {project.talkingPoints.length > 0 && (
                  <div>
                    <p className="text-xs text-slate-400 mb-1">Talking points</p>
                    <ul className="text-xs text-slate-300 space-y-0.5 pl-4 list-disc">
                      {project.talkingPoints.map((point) => (
                        <li key={`point-${idx}-${point}`}>{point}</li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Bottom Buttons */}
      <div className="border-t border-slate-700 p-4 space-y-2">
        {copyNotification && (
          <p className="text-xs text-emerald-400 mb-2">Copied to clipboard!</p>
        )}
        <button
          onClick={handleCopyToClipboard}
          className="w-full bg-blue-600 hover:bg-blue-500 text-white rounded-lg py-2 text-xs font-medium transition-colors"
        >
          Copy to clipboard
        </button>
        <button
          onClick={handleOpenResume}
          className="w-full bg-slate-700 hover:bg-slate-600 text-white rounded-lg py-2 text-xs font-medium transition-colors"
        >
          Open resume
        </button>
      </div>
    </div>
  );
}

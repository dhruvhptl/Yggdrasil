import { ExternalLink } from "lucide-react";

export interface SourceCardProps {
  index: number;
  title: string;
  url?: string | null;
  snippet?: string;
}

export function SourceCard({ index, title, url, snippet }: SourceCardProps) {
  const body = (
    <div className="rounded-md border border-white/10 bg-white/5 px-2.5 py-1.5 hover:bg-white/10 transition-colors">
      <div className="flex items-center gap-1.5 text-xs">
        <span className="font-mono opacity-50">[{index}]</span>
        <span className="font-medium truncate">{title}</span>
        {url && <ExternalLink className="w-3 h-3 opacity-50 shrink-0" />}
      </div>
      {snippet && <p className="mt-0.5 text-xs opacity-60 line-clamp-2">{snippet}</p>}
    </div>
  );
  return url ? <a href={url} target="_blank" rel="noreferrer" className="block">{body}</a> : body;
}

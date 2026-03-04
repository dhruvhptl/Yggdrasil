// src/layouts/MainLayout.tsx
import { ReactNode, useState } from "react";
import { NavLink } from "react-router-dom";
import { TreePine, Home, ListChecks, Library, Briefcase, ClipboardList, Lightbulb, FileText, Sparkles } from "lucide-react";
import MimirChat from "../components/MimirChat";

interface MainLayoutProps {
  children: ReactNode;
}

export default function MainLayout({ children }: MainLayoutProps) {
  const [chatOpen, setChatOpen] = useState(false);

  return (
    <div className="h-screen flex bg-slate-950 text-slate-100" style={{ position: "relative" }}>
      {/* Sidebar */}
      <aside className="w-64 border-r border-slate-800 bg-slate-900/60 backdrop-blur-sm flex flex-col">
        <div className="px-4 py-4 border-b border-slate-800 flex items-center gap-2">
          <TreePine className="w-6 h-6 text-emerald-400" />
          <div>
            <div className="text-sm font-semibold tracking-wide uppercase text-slate-300">
              Yggdrasil
            </div>
            <div className="text-xs text-slate-500">
              Universal skill tree
            </div>
          </div>
        </div>

        <nav className="flex-1 px-2 py-3 space-y-1">
          <SidebarLink to="/" icon={<Home className="w-4 h-4" />} label="Home" />
          <SidebarLink to="/trees" icon={<TreePine className="w-4 h-4" />} label="Trees" />
          <SidebarLink to="/quests" icon={<ListChecks className="w-4 h-4" />} label="Quests" />
          <SidebarLink to="/resources" icon={<Library className="w-4 h-4" />} label="Library" />
          <SidebarLink to="/work" icon={<Briefcase className="w-4 h-4" />} label="Work" />
          <SidebarLink to="/jobs" icon={<ClipboardList className="w-4 h-4" />} label="Jobs" />
          <SidebarLink to="/ideas" icon={<Lightbulb className="w-4 h-4" />} label="Ideas" />
          <SidebarLink to="/resume" icon={<FileText className="w-4 h-4" />} label="Resume" />
          <SidebarLink to="/skills" icon={<Sparkles className="w-4 h-4" />} label="Skills" />
        </nav>

        <div className="px-4 py-3 text-xs text-slate-600 border-t border-slate-800">
          v0.1 • Local prototype
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto bg-slate-950">
        {children}
      </main>

      {/* Mimir Chat sidebar */}
      <MimirChat open={chatOpen} onToggle={() => setChatOpen(v => !v)} />

      {/* Floating toggle button */}
      {!chatOpen && (
        <button
          onClick={() => setChatOpen(true)}
          style={{
            position: "fixed",
            bottom: 20,
            right: 20,
            zIndex: 50,
            width: 44,
            height: 44,
            borderRadius: 12,
            background: "rgba(15, 23, 42, 0.85)",
            backdropFilter: "blur(8px)",
            border: "1px solid #1e293b",
            color: "#94a3b8",
            cursor: "pointer",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontSize: 20,
            boxShadow: "0 4px 12px rgba(0,0,0,0.3)",
            transition: "border-color 0.15s, color 0.15s",
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.borderColor = "#10b981";
            e.currentTarget.style.color = "#10b981";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.borderColor = "#1e293b";
            e.currentTarget.style.color = "#94a3b8";
          }}
          title="Ask Mimir"
        >
          🔮
        </button>
      )}
    </div>
  );
}

interface SidebarLinkProps {
  to: string;
  icon: ReactNode;
  label: string;
}

function SidebarLink({ to, icon, label }: SidebarLinkProps) {
  return (
    <NavLink
      to={to}
      end={to === "/"}
      className={({ isActive }) =>
        [
          "group flex items-center gap-2 px-3 py-2 rounded-md text-sm font-medium transition-colors",
          isActive
            ? "bg-slate-800 text-emerald-300"
            : "text-slate-300 hover:bg-slate-800/60 hover:text-emerald-200",
        ].join(" ")
      }
    >
      <span className="text-slate-400 group-hover:text-emerald-300">
        {icon}
      </span>
      <span>{label}</span>
    </NavLink>
  );
}

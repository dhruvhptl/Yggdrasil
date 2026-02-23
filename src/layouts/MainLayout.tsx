// src/layouts/MainLayout.tsx
import { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { TreePine, Home, ListChecks, Library } from "lucide-react";

interface MainLayoutProps {
  children: ReactNode;
}

export default function MainLayout({ children }: MainLayoutProps) {
  return (
    <div className="h-screen flex bg-slate-950 text-slate-100">
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
        </nav>

        <div className="px-4 py-3 text-xs text-slate-600 border-t border-slate-800">
          v0.1 • Local prototype
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto bg-slate-950">
        {children}
      </main>
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

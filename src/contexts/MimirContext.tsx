import { createContext, useContext, useState, useRef, ReactNode } from 'react';

interface MimirContextValue {
  treeId: string | null;
  nodeId: string | null;
  nodeTitle: string | null;
  projectName: string | null;
  treeName: string | null;
  projectRootId: string | null;
  projectLabel: string | null;
  mode: 'chat' | 'dive';
  setMimirContext: (ctx: { treeId?: string | null; nodeId?: string | null; nodeTitle?: string | null; projectName?: string | null; treeName?: string | null; projectRootId?: string | null; projectLabel?: string | null; mode?: 'chat' | 'dive' }) => void;
  openMimir: () => void;
  registerOpenMimir: (fn: () => void) => void;
}

const MimirContext = createContext<MimirContextValue>({
  treeId: null,
  nodeId: null,
  nodeTitle: null,
  projectName: null,
  treeName: null,
  projectRootId: null,
  projectLabel: null,
  mode: 'chat',
  setMimirContext: () => {},
  openMimir: () => {},
  registerOpenMimir: () => {},
});

export function MimirProvider({ children }: { children: ReactNode }) {
  const [treeId, setTreeId] = useState<string | null>(null);
  const [nodeId, setNodeId] = useState<string | null>(null);
  const [nodeTitle, setNodeTitle] = useState<string | null>(null);
  const [projectName, setProjectName] = useState<string | null>(null);
  const [treeName, setTreeName] = useState<string | null>(null);
  const [projectRootId, setProjectRootId] = useState<string | null>(null);
  const [projectLabel, setProjectLabel] = useState<string | null>(null);
  const [mode, setMode] = useState<'chat' | 'dive'>('chat');
  const openMimirRef = useRef<() => void>(() => {});

  function setMimirContext(ctx: { treeId?: string | null; nodeId?: string | null; nodeTitle?: string | null; projectName?: string | null; treeName?: string | null; projectRootId?: string | null; projectLabel?: string | null; mode?: 'chat' | 'dive' }) {
    if (ctx.treeId !== undefined) setTreeId(ctx.treeId);
    if (ctx.nodeId !== undefined) setNodeId(ctx.nodeId);
    if (ctx.nodeTitle !== undefined) setNodeTitle(ctx.nodeTitle);
    if (ctx.projectName !== undefined) setProjectName(ctx.projectName);
    if (ctx.treeName !== undefined) setTreeName(ctx.treeName);
    if (ctx.projectRootId !== undefined) setProjectRootId(ctx.projectRootId);
    if (ctx.projectLabel !== undefined) setProjectLabel(ctx.projectLabel);
    if (ctx.mode !== undefined) setMode(ctx.mode);
  }

  function openMimir() {
    openMimirRef.current();
  }

  function registerOpenMimir(fn: () => void) {
    openMimirRef.current = fn;
  }

  return (
    <MimirContext.Provider value={{ treeId, nodeId, nodeTitle, projectName, treeName, projectRootId, projectLabel, mode, setMimirContext, openMimir, registerOpenMimir }}>
      {children}
    </MimirContext.Provider>
  );
}

export function useMimirContext() {
  return useContext(MimirContext);
}

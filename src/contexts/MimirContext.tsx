import { createContext, useContext, useState, useRef, ReactNode } from 'react';

interface MimirContextValue {
  treeId: string | null;
  nodeTitle: string | null;
  setMimirContext: (ctx: { treeId?: string | null; nodeTitle?: string | null }) => void;
  openMimir: () => void;
  registerOpenMimir: (fn: () => void) => void;
}

const MimirContext = createContext<MimirContextValue>({
  treeId: null,
  nodeTitle: null,
  setMimirContext: () => {},
  openMimir: () => {},
  registerOpenMimir: () => {},
});

export function MimirProvider({ children }: { children: ReactNode }) {
  const [treeId, setTreeId] = useState<string | null>(null);
  const [nodeTitle, setNodeTitle] = useState<string | null>(null);
  const openMimirRef = useRef<() => void>(() => {});

  function setMimirContext(ctx: { treeId?: string | null; nodeTitle?: string | null }) {
    if (ctx.treeId !== undefined) setTreeId(ctx.treeId);
    if (ctx.nodeTitle !== undefined) setNodeTitle(ctx.nodeTitle);
  }

  function openMimir() {
    openMimirRef.current();
  }

  function registerOpenMimir(fn: () => void) {
    openMimirRef.current = fn;
  }

  return (
    <MimirContext.Provider value={{ treeId, nodeTitle, setMimirContext, openMimir, registerOpenMimir }}>
      {children}
    </MimirContext.Provider>
  );
}

export function useMimirContext() {
  return useContext(MimirContext);
}

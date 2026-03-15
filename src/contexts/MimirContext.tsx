import { createContext, useContext, useState, ReactNode } from 'react';

interface MimirContextValue {
  treeId: string | null;
  nodeTitle: string | null;
  setMimirContext: (ctx: { treeId?: string | null; nodeTitle?: string | null }) => void;
}

const MimirContext = createContext<MimirContextValue>({
  treeId: null,
  nodeTitle: null,
  setMimirContext: () => {},
});

export function MimirProvider({ children }: { children: ReactNode }) {
  const [treeId, setTreeId] = useState<string | null>(null);
  const [nodeTitle, setNodeTitle] = useState<string | null>(null);

  function setMimirContext(ctx: { treeId?: string | null; nodeTitle?: string | null }) {
    if (ctx.treeId !== undefined) setTreeId(ctx.treeId);
    if (ctx.nodeTitle !== undefined) setNodeTitle(ctx.nodeTitle);
  }

  return (
    <MimirContext.Provider value={{ treeId, nodeTitle, setMimirContext }}>
      {children}
    </MimirContext.Provider>
  );
}

export function useMimirContext() {
  return useContext(MimirContext);
}

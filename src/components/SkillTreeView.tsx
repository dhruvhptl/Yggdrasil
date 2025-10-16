import React from 'react';
import ReactFlow, { Background, Controls, Node, Edge } from 'reactflow';
import 'reactflow/dist/style.css';

interface SkillTreeViewProps {
  skillTree: {
    nodes: Node[];
    edges: Edge[];
  };
}

export default function SkillTreeView({ skillTree }: SkillTreeViewProps) {
  return (
    <div style={{ width: '100%', height: '100%' }}>
      <ReactFlow
        nodes={skillTree.nodes}
        edges={skillTree.edges}
        fitView
      >
        <Background />
        <Controls />
      </ReactFlow>
    </div>
  );
}

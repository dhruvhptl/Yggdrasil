// src/components/SkillTree/TapeSkillTree.jsx

import React, { useState } from 'react';
import Tree from 'react-d3-tree';
import { tapeSkillTree } from './skillTreeData';
import './SkillTree.css';

const TapeSkillTree = () => {
  const [translate] = useState({ x: 400, y: 50 });

  // Custom node rendering based on status
  const renderCustomNode = ({ nodeDatum }) => {
    const status = nodeDatum.attributes?.status || 'locked';
    
    return (
      <g>
        <circle
          r={15}
          className={`node-circle node-${status}`}
        />
        <text
          fill="black"
          strokeWidth="0"
          x="25"
          y="5"
          className="node-text"
        >
          {nodeDatum.name}
        </text>
      </g>
    );
  };

  // Custom path styling based on node status
  const getPathClass = (linkData) => {
    const targetStatus = linkData.target.data.attributes?.status;
    return targetStatus === 'completed' || targetStatus === 'in-progress' 
      ? 'link-active' 
      : 'link-locked';
  };

  return (
    <div className="skill-tree-container">
      <Tree
        data={tapeSkillTree}
        orientation="vertical"
        pathFunc="diagonal"
        translate={translate}
        nodeSize={{ x: 200, y: 120 }}
        separation={{ siblings: 1.5, nonSiblings: 2 }}
        renderCustomNodeElement={renderCustomNode}
        pathClassFunc={getPathClass}
        enableLegacyTransitions={true}
        transitionDuration={500}
        zoom={0.8}
      />
    </div>
  );
};

export default TapeSkillTree;

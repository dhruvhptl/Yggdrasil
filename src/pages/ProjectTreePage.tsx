// src/pages/ProjectView.jsx or wherever you want to display it

import React from 'react';
// @ts-ignore: no declaration file for this JS module
import TapeSkillTree from '../components/SkillTree/TapeSkillTree.jsx';

function ProjectView() {
  return (
    <div>
      <h1>Skill Tree: tape</h1>
      <TapeSkillTree />
    </div>
  );
}

export default ProjectView;

import { useParams, useNavigate } from 'react-router-dom';
import SkillTreeView from '../components/SkillTreeView';
import { tapeSkillTree } from '../data/tapeSkillTree';

export default function ProjectTreePage() {
  const { projectName } = useParams<{ projectName: string }>();
  const navigate = useNavigate();

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100vh', padding: '16px' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '16px' }}>
        <h1 style={{ fontSize: '30px', fontWeight: 'bold' }}>Skill Tree: {projectName}</h1>
        <button 
          onClick={() => navigate('/')}
          style={{ padding: '8px 16px', backgroundColor: '#4B5563', color: 'white', border: 'none', borderRadius: '4px', cursor: 'pointer' }}
        >
          ← Back to Projects
        </button>
      </div>
      
      <div style={{ flex: 1, border: '1px solid #ddd', borderRadius: '8px', overflow: 'hidden' }}>
        <SkillTreeView skillTree={tapeSkillTree} />
      </div>
    </div>
  );
}

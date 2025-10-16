import { Node, Edge } from 'reactflow';

interface SkillTree {
  nodes: Node[];
  edges: Edge[];
}

export const tapeSkillTree: SkillTree = {
  nodes: [
    // Physics Foundation Branch
    { id: '1', position: { x: 250, y: 0 }, data: { label: 'Classical Mechanics' }, type: 'input' },
    { id: '2', position: { x: 100, y: 100 }, data: { label: 'Newtonian Physics' } },
    { id: '3', position: { x: 250, y: 100 }, data: { label: 'Lagrangian Mechanics' } },
    { id: '4', position: { x: 400, y: 100 }, data: { label: 'Hamiltonian Mechanics' } },
    { id: '5', position: { x: 250, y: 200 }, data: { label: 'Special Relativity' } },
    
    // Programming Branch
    { id: '6', position: { x: 600, y: 0 }, data: { label: 'Python Fundamentals' }, type: 'input' },
    { id: '7', position: { x: 500, y: 100 }, data: { label: 'NumPy' } },
    { id: '8', position: { x: 650, y: 100 }, data: { label: 'SciPy' } },
    { id: '9', position: { x: 800, y: 100 }, data: { label: 'Matplotlib' } },
    { id: '10', position: { x: 650, y: 200 }, data: { label: 'Numerical Methods' } },
    
    // ML Branch
    { id: '11', position: { x: 1000, y: 0 }, data: { label: 'ML Fundamentals' }, type: 'input' },
    { id: '12', position: { x: 1000, y: 100 }, data: { label: 'Neural Networks' } },
    { id: '13', position: { x: 1000, y: 200 }, data: { label: 'Time Series Prediction' } },
    { id: '14', position: { x: 1000, y: 300 }, data: { label: 'Physics-Informed NNs' } },
    
    // Integration
    { id: '15', position: { x: 650, y: 400 }, data: { label: 'Build Physics Engine' }, type: 'output' },
  ],
  edges: [
    // Physics connections
    { id: 'e1-2', source: '1', target: '2' },
    { id: 'e1-3', source: '1', target: '3' },
    { id: 'e1-4', source: '1', target: '4' },
    { id: 'e3-5', source: '3', target: '5' },
    
    // Programming connections
    { id: 'e6-7', source: '6', target: '7' },
    { id: 'e6-8', source: '6', target: '8' },
    { id: 'e6-9', source: '6', target: '9' },
    { id: 'e7-10', source: '7', target: '10' },
    { id: 'e8-10', source: '8', target: '10' },
    
    // ML connections
    { id: 'e11-12', source: '11', target: '12' },
    { id: 'e12-13', source: '12', target: '13' },
    { id: 'e13-14', source: '13', target: '14' },
    
    // Final connections to project
    { id: 'e5-15', source: '5', target: '15' },
    { id: 'e10-15', source: '10', target: '15' },
    { id: 'e14-15', source: '14', target: '15' },
  ]
};

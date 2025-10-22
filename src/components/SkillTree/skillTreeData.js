// src/components/SkillTree/skillTreeData.js

export const tapeSkillTree = {
  name: 'TAPE Project',
  children: [
    {
      name: 'Classical Mechanics',
      attributes: { status: 'locked' },
      children: [
        {
          name: 'Newtonian Physics',
          attributes: { status: 'locked' }
        },
        {
          name: 'Lagrangian Mechanics',
          attributes: { status: 'locked' },
          children: [
            {
              name: 'Hamiltonian Mechanics',
              attributes: { status: 'locked' },
              children: [
                { 
                  name: 'Special Relativity', 
                  attributes: { status: 'locked' } 
                }
              ]
            }
          ]
        }
      ]
    },
    {
      name: 'Python Fundamentals',
      attributes: { status: 'completed' },
      children: [
        {
          name: 'NumPy',
          attributes: { status: 'in-progress' },
          children: [
            {
              name: 'Numerical Methods',
              attributes: { status: 'locked' },
              children: [
                { 
                  name: 'Build Physics Engine', 
                  attributes: { status: 'locked' } 
                }
              ]
            }
          ]
        },
        { 
          name: 'SciPy', 
          attributes: { status: 'locked' },
          children: [
            {
              name: 'Numerical Methods',
              attributes: { status: 'locked' }
            }
          ]
        },
        { 
          name: 'Matplotlib', 
          attributes: { status: 'locked' } 
        }
      ]
    },
    {
      name: 'ML Fundamentals',
      attributes: { status: 'locked' },
      children: [
        {
          name: 'Neural Networks',
          attributes: { status: 'locked' },
          children: [
            { 
              name: 'Time Series Prediction', 
              attributes: { status: 'locked' } 
            },
            { 
              name: 'Physics-Informed NNs', 
              attributes: { status: 'locked' },
              children: [
                {
                  name: 'Build Physics Engine',
                  attributes: { status: 'locked' }
                }
              ]
            }
          ]
        }
      ]
    }
  ]
};

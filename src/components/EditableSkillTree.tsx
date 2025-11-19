// src/components/EditableSkillTree.tsx

import React, { useState, useEffect } from 'react';
import Tree from 'react-d3-tree';
import { invoke } from '@tauri-apps/api/core';
import "./SkillTree/SkillTree.css";

interface TreeNode {
  id: string;
  tree_id: string;
  parent_id: string | null;
  type: 'trunk' | 'branch' | 'leaf';
  title: string;
  description: string;
  progress: number;
  tasks: any;
  resources: string | null;
  x: number | null;
  y: number | null;
  order_index: number;
}

interface TreeData {
  name: string;
  attributes?: {
    id: string;
    description: string;
    progress: number;
    type: string;
  };
  children?: TreeData[];
}

interface EditableSkillTreeProps {
  projectId: string;
}

export default function EditableSkillTree({ projectId }: EditableSkillTreeProps) {
  const [treeId, setTreeId] = useState<string | null>(null);
  const [treeData, setTreeData] = useState<TreeData | null>(null);
  const [nodes, setNodes] = useState<TreeNode[]>([]);
  const [selectedNode, setSelectedNode] = useState<TreeNode | null>(null);
  const [isEditing, setIsEditing] = useState(false);
  const [translate] = useState({ x: 400, y: 50 });

  useEffect(() => {
    loadTree();
  }, [projectId]);

  const loadTree = async () => {
    try {
      const trees = await invoke<any[]>('get_trees', { projectId: projectId });
      
      let currentTreeId: string;
      
      if (trees.length === 0) {
        const newTree = await invoke<any>('create_tree', {
          projectId: projectId,
          name: `${projectId} Skills`
        });
        currentTreeId = newTree.id;
        
        await invoke('create_tree_node', {
          treeId: currentTreeId,
          parentId: null,
          type: 'trunk',
          title: 'Start Here',
          description: 'Root of your skill tree'
        });
      } else {
        currentTreeId = trees[0].id;
      }
      
      setTreeId(currentTreeId);
      await loadTreeContents(currentTreeId);
    } catch (error) {
      console.error('Failed to load tree:', error);
    }
  };

  const loadTreeContents = async (id: string) => {
    try {
      const [tree, nodeList, edges] = await invoke<[any, TreeNode[], any[]]>(
        'get_tree_with_contents',
        { treeId: id }
      );
      
      setNodes(nodeList);
      const treeStructure = buildTreeStructure(nodeList);
      setTreeData(treeStructure);
    } catch (error) {
      console.error('Failed to load tree contents:', error);
    }
  };

  const buildTreeStructure = (nodeList: TreeNode[]): TreeData => {
    const nodeMap = new Map<string, TreeData>();
    
    nodeList.forEach(node => {
      nodeMap.set(node.id, {
        name: node.title,
        attributes: {
          id: node.id,
          description: node.description,
          progress: node.progress,
          type: node.type
        },
        children: []
      });
    });
    
    let root: TreeData | null = null;
    nodeList.forEach(node => {
      const treeNode = nodeMap.get(node.id)!;
      if (node.parent_id === null) {
        root = treeNode;
      } else {
        const parent = nodeMap.get(node.parent_id);
        if (parent && parent.children) {
          parent.children.push(treeNode);
        }
      }
    });
    
    return root || { name: 'Empty Tree', children: [] };
  };

  const handleNodeClick = (nodeDatum: any) => {
    const node = nodes.find(n => n.id === nodeDatum.attributes?.id);
    if (node) {
      setSelectedNode(node);
      setIsEditing(true);
    }
  };

  const handleUpdateNode = async (updates: Partial<TreeNode>) => {
    if (!selectedNode) return;
    
    try {
      await invoke('update_tree_node', {
        nodeId: selectedNode.id,
        title: updates.title,
        description: updates.description,
        progress: updates.progress,
        tasks: updates.tasks,
        resources: updates.resources  // ADD THIS LINE
      });
      
      if (treeId) {
        await loadTreeContents(treeId);
      }
      setIsEditing(false);
      setSelectedNode(null);
    } catch (error) {
      console.error('Failed to update node:', error);
    }
  };

  const handleAddChild = async () => {
    if (!selectedNode || !treeId) return;
    
    try {
      await invoke('create_tree_node', {
        treeId,
        parentId: selectedNode.id,
        type: selectedNode.type === 'trunk' ? 'branch' : 'leaf',
        title: 'New Skill',
        description: 'Click to edit'
      });
      
      await loadTreeContents(treeId);
    } catch (error) {
      console.error('Failed to add child node:', error);
    }
  };

  // ADD THIS NEW FUNCTION
  const handleDeleteNode = async () => {
    if (!selectedNode || !treeId) return;
    
    // Don't allow deleting the root node
    if (selectedNode.parent_id === null) {
      alert("Cannot delete the root node!");
      return;
    }
    
    // Confirm before deleting
    if (!window.confirm(`Delete "${selectedNode.title}" and all its children?`)) {
      return;
    }
    
    try {
      await invoke('delete_tree_node', {
        nodeId: selectedNode.id
      });
      
      // Reload tree
      await loadTreeContents(treeId);
      setIsEditing(false);
      setSelectedNode(null);
    } catch (error) {
      console.error('Failed to delete node:', error);
      alert('Failed to delete node: ' + error);
    }
  };
  
  const parseResources = (resourceString: string | null): string[] => {
    if (!resourceString) return [];
    try {
      return JSON.parse(resourceString);
    } catch {
      return [];
    }
  };

  const renderCustomNode = ({ nodeDatum, toggleNode }: any) => {
    const progress = nodeDatum.attributes?.progress || 0;
    const nodeType = nodeDatum.attributes?.type || 'branch';
    
    return (
      <g onClick={() => handleNodeClick(nodeDatum)}>
        <circle
          r={20}
          fill={progress === 100 ? '#10b981' : progress > 0 ? '#3b82f6' : '#6b7280'}
          stroke="#fff"
          strokeWidth="2"
          style={{ cursor: 'pointer' }}
        />
        <text
          fill="white"
          strokeWidth="0"
          x="0"
          y="5"
          textAnchor="middle"
          style={{ fontSize: '12px', fontWeight: 'bold' }}
        >
          {progress}%
        </text>
        <text
          fill="black"
          strokeWidth="0"
          x="30"
          y="5"
          style={{ fontSize: '14px', fontWeight: '500', cursor: 'pointer' }}
        >
          {nodeDatum.name}
        </text>
        <text
          fill="#6b7280"
          strokeWidth="0"
          x="30"
          y="20"
          style={{ fontSize: '10px' }}
        >
          {nodeType}
        </text>
      </g>
    );
  };

  return (
    <div style={{ width: '100%', height: '100vh', position: 'relative' }}>
      {treeData && (
        <Tree
          data={treeData}
          orientation="vertical"
          pathFunc="diagonal"
          translate={translate}
          nodeSize={{ x: 200, y: 120 }}
          separation={{ siblings: 1.5, nonSiblings: 2 }}
          renderCustomNodeElement={renderCustomNode}
          enableLegacyTransitions={true}
          transitionDuration={500}
          zoom={0.8}
        />
      )}
      
      {/* Edit Panel */}
{isEditing && selectedNode && (
  <div style={{
    position: 'absolute',
    top: 20,
    right: 20,
    background: 'white',
    padding: '20px',
    borderRadius: '8px',
    boxShadow: '0 4px 6px rgba(0,0,0,0.1)',
    width: '350px',
    maxHeight: '80vh',
    overflowY: 'auto',
    zIndex: 1000
  }}>
    <h3 style={{ marginTop: 0 }}>Edit Skill</h3>
    
    <div style={{ marginBottom: '15px' }}>
      <label style={{ display: 'block', marginBottom: '5px', fontWeight: '500' }}>
        Title
      </label>
      <input
        type="text"
        value={selectedNode.title}
        onChange={(e) => setSelectedNode({ ...selectedNode, title: e.target.value })}
        style={{
          width: '100%',
          padding: '8px',
          border: '1px solid #d1d5db',
          borderRadius: '4px'
        }}
      />
    </div>
    
    <div style={{ marginBottom: '15px' }}>
      <label style={{ display: 'block', marginBottom: '5px', fontWeight: '500' }}>
        Description
      </label>
      <textarea
        value={selectedNode.description}
        onChange={(e) => setSelectedNode({ ...selectedNode, description: e.target.value })}
        rows={3}
        style={{
          width: '100%',
          padding: '8px',
          border: '1px solid #d1d5db',
          borderRadius: '4px'
        }}
      />
    </div>
    
    <div style={{ marginBottom: '15px' }}>
      <label style={{ display: 'block', marginBottom: '5px', fontWeight: '500' }}>
        Progress: {selectedNode.progress}%
      </label>
      <input
        type="range"
        min="0"
        max="100"
        value={selectedNode.progress}
        onChange={(e) => setSelectedNode({ ...selectedNode, progress: parseInt(e.target.value) })}
        style={{ width: '100%' }}
      />
    </div>
    
    {/* RESOURCES SECTION - NEW */}
    <div style={{ marginBottom: '15px' }}>
      <label style={{ display: 'block', marginBottom: '5px', fontWeight: '500' }}>
        Resources
      </label>
      {parseResources(selectedNode.resources).map((url, index) => (
        <div key={index} style={{ display: 'flex', gap: '5px', marginBottom: '5px' }}>
          <input
            type="text"
            value={url}
            onChange={(e) => {
              const resources = parseResources(selectedNode.resources);
              resources[index] = e.target.value;
              setSelectedNode({ 
                ...selectedNode, 
                resources: JSON.stringify(resources) 
              });
            }}
            placeholder="https://..."
            style={{
              flex: 1,
              padding: '6px',
              border: '1px solid #d1d5db',
              borderRadius: '4px',
              fontSize: '12px'
            }}
          />
          <button
            onClick={() => {
              const resources = parseResources(selectedNode.resources);
              resources.splice(index, 1);
              setSelectedNode({ 
                ...selectedNode, 
                resources: JSON.stringify(resources) 
              });
            }}
            style={{
              padding: '6px 10px',
              background: '#ef4444',
              color: 'white',
              border: 'none',
              borderRadius: '4px',
              cursor: 'pointer',
              fontSize: '12px'
            }}
          >
            ✕
          </button>
        </div>
      ))}
      <button
        onClick={() => {
          const resources = parseResources(selectedNode.resources);
          resources.push('');
          setSelectedNode({ 
            ...selectedNode, 
            resources: JSON.stringify(resources) 
          });
        }}
        style={{
          width: '100%',
          padding: '6px',
          background: '#e5e7eb',
          color: '#374151',
          border: '1px solid #d1d5db',
          borderRadius: '4px',
          cursor: 'pointer',
          fontSize: '12px'
        }}
      >
        + Add Resource Link
      </button>
      
      {/* Display clickable links */}
      {parseResources(selectedNode.resources).length > 0 && (
        <div style={{ marginTop: '10px', paddingTop: '10px', borderTop: '1px solid #e5e7eb' }}>
          <strong style={{ fontSize: '12px', color: '#6b7280' }}>Quick Links:</strong>
          <div style={{ marginTop: '5px' }}>
            {parseResources(selectedNode.resources).filter(url => url.trim()).map((url, index) => (
              <a
                key={index}
                href={url}
                target="_blank"
                rel="noopener noreferrer"
                style={{
                  display: 'block',
                  padding: '4px 8px',
                  marginBottom: '4px',
                  background: '#eff6ff',
                  color: '#2563eb',
                  textDecoration: 'none',
                  borderRadius: '4px',
                  fontSize: '11px',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap'
                }}
              >
                🔗 {url}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
    
    <div style={{ display: 'flex', gap: '10px', marginBottom: '10px' }}>
      <button
        onClick={() => handleUpdateNode(selectedNode)}
        style={{
          flex: 1,
          padding: '8px',
          background: '#3b82f6',
          color: 'white',
          border: 'none',
          borderRadius: '4px',
          cursor: 'pointer'
        }}
      >
        Save
      </button>
      <button
        onClick={handleAddChild}
        style={{
          flex: 1,
          padding: '8px',
          background: '#10b981',
          color: 'white',
          border: 'none',
          borderRadius: '4px',
          cursor: 'pointer'
        }}
      >
        Add Child
      </button>
    </div>
    
    <div style={{ display: 'flex', gap: '10px' }}>
      <button
        onClick={handleDeleteNode}
        style={{
          flex: 1,
          padding: '8px',
          background: '#ef4444',
          color: 'white',
          border: 'none',
          borderRadius: '4px',
          cursor: 'pointer'
        }}
      >
        Delete
      </button>
      <button
        onClick={() => {
          setIsEditing(false);
          setSelectedNode(null);
        }}
        style={{
          flex: 1,
          padding: '8px',
          background: '#6b7280',
          color: 'white',
          border: 'none',
          borderRadius: '4px',
          cursor: 'pointer'
        }}
      >
        Cancel
      </button>
    </div>
  </div>
)}
    </div>
  );
} 

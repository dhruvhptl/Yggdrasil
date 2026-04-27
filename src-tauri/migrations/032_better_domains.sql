-- migrations/032_better_domains.sql
-- Seed more specific sub-domains for better AI classification granularity.
-- Adds a description column to skill_domains if not present, then upserts
-- all new domains. Existing broad domains from 029 are preserved.

ALTER TABLE skill_domains ADD COLUMN IF NOT EXISTS description TEXT;

INSERT INTO skill_domains (id, name, description)
VALUES
    ('dom-ml',          'Machine Learning',              'Algorithms that learn from data — regression, classification, clustering, optimization, model evaluation'),
    ('dom-dl',          'Deep Learning',                 'Neural networks, backprop, CNNs, RNNs, transformers, attention, training at scale'),
    ('dom-nlp',         'Natural Language Processing',   'Text processing, tokenization, embeddings, language models, parsing, sentiment, translation'),
    ('dom-cv',          'Computer Vision',               'Image/video understanding, object detection, segmentation, optical flow, 3D vision'),
    ('dom-quantum',     'Quantum Computing',             'Quantum circuits, qubits, gates, entanglement, quantum algorithms and complexity'),
    ('dom-systems',     'Systems Programming',           'OS internals, memory management, concurrency, low-level languages (C, Rust, Zig), performance'),
    ('dom-web',         'Web Development',               'Frontend (HTML/CSS/JS, React, Vue), backend APIs, HTTP, authentication, deployment pipelines'),
    ('dom-datascience', 'Data Science',                  'Data wrangling, EDA, feature engineering, statistical analysis, notebooks, pipelines'),
    ('dom-physics',     'Physics',                       'Classical mechanics, electromagnetism, quantum mechanics, thermodynamics, relativity, simulations'),
    ('dom-softeng',     'Software Engineering',          'Design patterns, architecture, testing, refactoring, code quality, system design, APIs'),
    ('dom-devops',      'DevOps',                        'CI/CD, containerization (Docker, K8s), infrastructure as code, monitoring, cloud platforms'),
    ('dom-databases',   'Databases',                     'Relational (SQL, PostgreSQL), NoSQL (Mongo, Redis), indexing, query optimization, schema design'),
    ('dom-viz',         'Visualization',                 'Data visualization, charting libraries (D3, Vega), dashboards, scientific plotting, rendering'),
    ('dom-research-methods', 'Research Methods',          'Experimental design, literature review, academic writing, citation, reproducibility, peer review')
ON CONFLICT (name) DO UPDATE SET description = EXCLUDED.description;

// src/App.tsx
import { BrowserRouter, Routes, Route } from "react-router-dom";
import MainLayout from "./layouts/MainLayout";
import HomePage from "./pages/Homepage";
import TreesPage from "./pages/TreesPage";
import QuestsPage from "./pages/QuestsPage";
import ResourcesPage from "./pages/ResourcesPage";
import ProjectTreePage from "./pages/ProjectTreePage";

export default function App() {
  return (
    <BrowserRouter>
      <MainLayout>
        <Routes>
          <Route path="/" element={<HomePage />} />
          <Route path="/trees" element={<TreesPage />} />
          <Route path="/quests" element={<QuestsPage />} />
          <Route path="/resources" element={<ResourcesPage />} />
          <Route path="/project/:projectId" element={<ProjectTreePage />} />
        </Routes>
      </MainLayout>
    </BrowserRouter>
  );
}

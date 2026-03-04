// src/App.tsx
import { BrowserRouter, Routes, Route } from "react-router-dom";
import MainLayout from "./layouts/MainLayout";
import HomePage from "./pages/Homepage";
import TreesPage from "./pages/TreesPage";
import QuestsPage from "./pages/QuestsPage";
import ResourcesPage from "./pages/ResourcesPage";
import WorkPage from "./pages/WorkPage";
import JobsPage from "./pages/JobsPage";
import IdeasPage from "./pages/IdeasPage";
import ResumePage from "./pages/ResumePage";
import SkillsPage from "./pages/SkillsPage";
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
          <Route path="/work" element={<WorkPage />} />
          <Route path="/jobs" element={<JobsPage />} />
          <Route path="/ideas" element={<IdeasPage />} />
          <Route path="/resume" element={<ResumePage />} />
          <Route path="/skills" element={<SkillsPage />} />
          <Route path="/project/:projectId" element={<ProjectTreePage />} />
        </Routes>
      </MainLayout>
    </BrowserRouter>
  );
}

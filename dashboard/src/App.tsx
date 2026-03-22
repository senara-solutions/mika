import { Routes, Route } from 'react-router'
import Layout from './components/Layout.tsx'
import Timeline from './pages/Timeline.tsx'
import TraceDetail from './pages/TraceDetail.tsx'
import Traces from './pages/Traces.tsx'
import Agents from './pages/Agents.tsx'
import AgentDetail from './pages/AgentDetail.tsx'
import Sessions from './pages/Sessions.tsx'
import SessionDetail from './pages/SessionDetail.tsx'
import Tasks from './pages/Tasks.tsx'
import DevRuns from './pages/DevRuns.tsx'
import DevRunDetail from './pages/DevRunDetail.tsx'
import TeamRuns from './pages/TeamRuns.tsx'
import TeamRunDetail from './pages/TeamRunDetail.tsx'
import NotFound from './pages/NotFound.tsx'

export default function App() {
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route index element={<Timeline />} />
        <Route path="traces" element={<Traces />} />
        <Route path="traces/:traceId" element={<TraceDetail />} />
        <Route path="agents" element={<Agents />} />
        <Route path="agents/:agentId" element={<AgentDetail />} />
        <Route path="sessions" element={<Sessions />} />
        <Route path="sessions/:sessionId" element={<SessionDetail />} />
        <Route path="tasks" element={<Tasks />} />
        <Route path="dev-runs" element={<DevRuns />} />
        <Route path="dev-runs/:taskId" element={<DevRunDetail />} />
        <Route path="team-runs" element={<TeamRuns />} />
        <Route path="team-runs/:runId" element={<TeamRunDetail />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  )
}

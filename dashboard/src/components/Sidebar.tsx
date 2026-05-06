import { NavLink } from 'react-router'
import { LayoutDashboard, Activity, Bot, MessageSquare, Search, CheckSquare, Hammer, Users, Brain, Wrench, Layers } from 'lucide-react'

const navItems = [
  { to: '/', label: 'Overview', icon: LayoutDashboard },
  { to: '/timeline', label: 'Event Timeline', icon: Activity },
  { to: '/agents', label: 'Agents', icon: Bot },
  { to: '/sessions', label: 'Sessions', icon: MessageSquare },
  { to: '/traces', label: 'Traces', icon: Search },
  { to: '/tasks', label: 'Tasks', icon: CheckSquare },
  { to: '/llm-calls', label: 'LLM Calls', icon: Brain },
  { to: '/tool-calls', label: 'Tool Calls', icon: Wrench },
  { to: '/dev-runs', label: 'Dev Runs', icon: Hammer },
  { to: '/team-runs', label: 'Team Runs', icon: Users },
  { to: '/skill-variants', label: 'Skill Variants', icon: Layers },
]

export default function Sidebar() {
  return (
    <aside className="w-56 shrink-0 border-r border-white/[0.05] bg-bg-card flex flex-col">
      <div className="p-5 border-b border-white/[0.05]">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-lg bg-accent/20 flex items-center justify-center">
            <span className="text-accent font-bold text-sm">M</span>
          </div>
          <div>
            <h1 className="text-heading font-bold text-sm tracking-tight">MikaPlatform</h1>
            <p className="text-[10px] text-muted/50 uppercase tracking-widest">Observability</p>
          </div>
        </div>
      </div>
      <nav className="flex-1 p-3 space-y-0.5">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.to === '/'}
            className={({ isActive }) =>
              `flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-colors ${
                isActive
                  ? 'bg-accent/10 text-accent-light font-medium'
                  : 'text-muted hover:text-heading hover:bg-white/[0.03]'
              }`
            }
          >
            <item.icon size={16} />
            {item.label}
          </NavLink>
        ))}
      </nav>
    </aside>
  )
}

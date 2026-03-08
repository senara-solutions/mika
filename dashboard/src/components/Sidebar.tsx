import { NavLink } from 'react-router'
import { Activity, Bot, MessageSquare } from 'lucide-react'

const navItems = [
  { to: '/', label: 'Timeline', icon: Activity },
  { to: '/agents', label: 'Agents', icon: Bot },
  { to: '/sessions', label: 'Sessions', icon: MessageSquare },
]

export default function Sidebar() {
  return (
    <aside className="w-56 shrink-0 border-r border-white/[0.05] bg-bg-card flex flex-col">
      <div className="p-5 border-b border-white/[0.05]">
        <h1 className="text-heading font-bold text-lg tracking-tight">Mika</h1>
        <p className="text-xs text-muted/60 mt-0.5">Dashboard</p>
      </div>
      <nav className="flex-1 p-3 space-y-1">
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

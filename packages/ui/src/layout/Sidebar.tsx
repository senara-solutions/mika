import type { ComponentType, ReactNode } from 'react'

export interface NavItem {
  to: string
  label: string
  icon: ComponentType<{ size?: number }>
}

export interface SidebarBrand {
  name: string
  subtitle: string
}

interface SidebarProps {
  brand: SidebarBrand
  navItems: NavItem[]
  renderLink: (item: NavItem, children: ReactNode) => ReactNode
}

export default function Sidebar({ brand, navItems, renderLink }: SidebarProps) {
  return (
    <aside className="w-56 shrink-0 border-r border-white/[0.05] bg-bg-card flex flex-col">
      <div className="p-5 border-b border-white/[0.05]">
        <div className="flex items-center gap-2.5">
          <div className="w-8 h-8 rounded-lg bg-accent/20 flex items-center justify-center">
            <span className="text-accent font-bold text-sm">M</span>
          </div>
          <div>
            <h1 className="text-heading font-bold text-sm tracking-tight">{brand.name}</h1>
            <p className="text-[10px] text-muted/50 uppercase tracking-widest">{brand.subtitle}</p>
          </div>
        </div>
      </div>
      <nav className="flex-1 p-3 space-y-0.5">
        {navItems.map((item) => (
          <div key={item.to}>
            {renderLink(item, (
              <>
                <item.icon size={16} />
                {item.label}
              </>
            ))}
          </div>
        ))}
      </nav>
    </aside>
  )
}

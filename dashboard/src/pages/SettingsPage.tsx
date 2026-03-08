import { Settings } from 'lucide-react'

export default function SettingsPage() {
  return (
    <div>
      <div className="mb-5">
        <h2 className="text-heading text-xl font-semibold">Settings</h2>
        <p className="text-sm text-muted/60 mt-1">Dashboard configuration</p>
      </div>

      <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-8 max-w-xl">
        <div className="flex flex-col items-center justify-center py-8 text-muted/40">
          <Settings size={32} className="mb-3" />
          <p className="text-sm">Settings coming soon</p>
        </div>
      </div>
    </div>
  )
}

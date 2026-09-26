export function MetadataRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3 py-2">
      <span className="text-muted/60 text-xs w-28 shrink-0 uppercase tracking-wider">{label}</span>
      <span className="text-heading text-sm">{children}</span>
    </div>
  )
}

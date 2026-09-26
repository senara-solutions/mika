import SelectFilter from './SelectFilter.tsx'

export interface AgentSummary {
  id: string
  name: string
}

export interface AgentFilterProps {
  agents: AgentSummary[] | undefined
  value: string
  onChange: (agentId: string) => void
  emptyLabel?: string
}

export default function AgentFilter({ agents, value, onChange, emptyLabel = 'All Agents' }: AgentFilterProps) {
  const options = [
    { label: emptyLabel, value: '' },
    ...(agents ?? []).map((a) => ({ label: a.name, value: a.id })),
  ]

  return (
    <SelectFilter
      ariaLabel="Filter by agent"
      value={value}
      onChange={onChange}
      options={options}
    />
  )
}

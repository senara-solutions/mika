/**
 * Deterministic color assignment from agent name.
 * Uses a hash to pick from a palette of distinguishable colors.
 */

const PALETTE = [
  { bg: 'bg-blue-500/15', text: 'text-blue-400', dot: 'bg-blue-400' },
  { bg: 'bg-emerald-500/15', text: 'text-emerald-400', dot: 'bg-emerald-400' },
  { bg: 'bg-purple-500/15', text: 'text-purple-400', dot: 'bg-purple-400' },
  { bg: 'bg-amber-500/15', text: 'text-amber-400', dot: 'bg-amber-400' },
  { bg: 'bg-pink-500/15', text: 'text-pink-400', dot: 'bg-pink-400' },
  { bg: 'bg-cyan-500/15', text: 'text-cyan-400', dot: 'bg-cyan-400' },
  { bg: 'bg-orange-500/15', text: 'text-orange-400', dot: 'bg-orange-400' },
  { bg: 'bg-indigo-500/15', text: 'text-indigo-400', dot: 'bg-indigo-400' },
  { bg: 'bg-rose-500/15', text: 'text-rose-400', dot: 'bg-rose-400' },
  { bg: 'bg-teal-500/15', text: 'text-teal-400', dot: 'bg-teal-400' },
]

function hashString(str: string): number {
  let hash = 0
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i)
    hash = ((hash << 5) - hash + char) | 0
  }
  return Math.abs(hash)
}

export function getAgentColor(agentName: string) {
  return PALETTE[hashString(agentName) % PALETTE.length]
}

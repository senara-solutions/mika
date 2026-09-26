import { Check } from 'lucide-react'

export type StepStatus = 'completed' | 'active' | 'pending'

export interface PipelineStep {
  label: string
  status: StepStatus
}

interface PipelineTimelineProps {
  steps: PipelineStep[]
}

function StepCircle({ status }: { status: StepStatus }) {
  if (status === 'completed') {
    return (
      <div className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-500/20 text-emerald-400">
        <Check className="h-3.5 w-3.5" />
      </div>
    )
  }
  if (status === 'active') {
    return (
      <div className="relative flex h-6 w-6 items-center justify-center">
        <div className="absolute h-6 w-6 animate-ping rounded-full bg-blue-500/20" />
        <div className="h-3 w-3 rounded-full bg-blue-400" />
      </div>
    )
  }
  return <div className="h-3 w-3 rounded-full bg-white/10" />
}

function Connector({ status }: { status: StepStatus }) {
  return (
    <div
      className={`mx-1 h-0.5 flex-1 ${
        status === 'completed' ? 'bg-emerald-500/30' : 'bg-white/[0.06]'
      }`}
    />
  )
}

export function PipelineTimeline({ steps }: PipelineTimelineProps) {
  return (
    <div className="bg-bg-card border border-white/[0.05] rounded-2xl p-5">
      <h3 className="text-heading text-sm font-medium mb-4">Pipeline</h3>
      <div className="flex items-center">
        {steps.map((step, i) => (
          <div key={step.label} className="contents">
            <div className="flex flex-col items-center gap-1.5">
              <StepCircle status={step.status} />
              <span
                className={`text-[10px] font-medium ${
                  step.status === 'completed'
                    ? 'text-emerald-400/80'
                    : step.status === 'active'
                      ? 'text-blue-400'
                      : 'text-muted/30'
                }`}
              >
                {step.label}
              </span>
            </div>
            {i < steps.length - 1 && (
              <Connector status={steps[i + 1].status === 'pending' ? 'pending' : 'completed'} />
            )}
          </div>
        ))}
      </div>
    </div>
  )
}

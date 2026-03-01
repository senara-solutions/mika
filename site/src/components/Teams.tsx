import { FadeIn } from "./FadeIn";
import { Users, GitBranch, FolderSync, Bot } from "lucide-react";

export function Teams() {
  return (
    <section id="teams" className="mx-auto max-w-7xl px-6 py-32 lg:py-40">
      <FadeIn className="mb-20 text-center">
        <h2 className="text-3xl font-extrabold tracking-tight text-white sm:text-4xl lg:text-5xl">
          One assistant is powerful.
          <br />
          <span className="text-accent">A team is unstoppable.</span>
        </h2>
        <p className="mx-auto mt-5 max-w-2xl text-lg text-muted">
          Create specialized agents with their own memory and personality, then
          compose them into teams that collaborate on complex goals.
        </p>
      </FadeIn>

      <div className="grid gap-6 lg:grid-cols-2">
        {/* Left: Orchestration diagram */}
        <FadeIn>
          <div className="h-full rounded-2xl border border-white/[0.05] bg-bg-card p-8 sm:p-10">
            <OrchestrationDiagram />
          </div>
        </FadeIn>

        {/* Right: Terminal showing team usage */}
        <FadeIn delay={100}>
          <div className="h-full overflow-hidden rounded-2xl border border-white/[0.06] bg-bg-card">
            {/* Title bar */}
            <div className="flex items-center gap-2 border-b border-white/[0.04] bg-white/[0.02] px-4 py-3">
              <div className="h-3 w-3 rounded-full bg-[#ff5f57]/60" />
              <div className="h-3 w-3 rounded-full bg-[#febc2e]/60" />
              <div className="h-3 w-3 rounded-full bg-[#28c840]/60" />
              <span className="ml-3 text-[11px] font-medium uppercase tracking-widest text-muted/40">
                team management
              </span>
            </div>
            <div className="p-6 font-mono text-[13px] leading-7 sm:text-sm sm:leading-8">
              <Line prompt="$" text="mika agents create researcher" />
              <Line
                prompt=""
                text="Agent 'researcher' created at ~/.mika/agents/researcher/"
                dim
              />
              <Line
                prompt="$"
                text='mika teams run dev-team "Analyze our Q3 metrics and draft a board summary"'
                className="mt-4"
              />
              <Line
                prompt=""
                text="Running team 'dev-team' (3 agents, max 3 iterations)"
                dim
                className="mt-2"
              />
              <Line
                prompt=""
                text="[decompose] planner → 2 tasks assigned"
                color="text-accent"
                className="mt-2"
              />
              <Line
                prompt=""
                text="[execute]   researcher → research.md ✓"
                color="text-emerald-400"
              />
              <Line
                prompt=""
                text="[execute]   writer → draft.md ✓"
                color="text-emerald-400"
              />
              <Line
                prompt=""
                text="[review]    critic → approved"
                color="text-amber-400"
              />
              <Line
                prompt=""
                text="[deliver]   → deliverable.md (2,847 words)"
                color="text-emerald-400"
              />
              <Line prompt="" text="Team run completed in 47s" dim className="mt-2" />
            </div>
          </div>
        </FadeIn>
      </div>

      {/* Capability cards below */}
      <div className="mt-6 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {[
          {
            icon: Bot,
            title: "Independent Agents",
            desc: "Each agent has its own memory, personality, and skills.",
          },
          {
            icon: GitBranch,
            title: "Hub-and-Spoke",
            desc: "Orchestrator decomposes. Specialists execute in parallel.",
          },
          {
            icon: FolderSync,
            title: "Shared Workspace",
            desc: "Agents collaborate through a shared filesystem.",
          },
          {
            icon: Users,
            title: "CLI Management",
            desc: "Create, clone, switch, and compose agents from the terminal.",
          },
        ].map((item, i) => (
          <FadeIn key={item.title} delay={i * 80}>
            <div className="group h-full rounded-2xl border border-white/[0.05] bg-bg-card p-6 transition-all duration-300 hover:border-accent/40 hover:shadow-[0_0_30px_rgba(124,106,247,0.08)]">
              <item.icon className="mb-4 h-5 w-5 text-accent" strokeWidth={2} />
              <h4 className="text-sm font-bold text-white">{item.title}</h4>
              <p className="mt-1.5 text-xs leading-relaxed text-muted/60">
                {item.desc}
              </p>
            </div>
          </FadeIn>
        ))}
      </div>
    </section>
  );
}

function Line({
  prompt,
  text,
  dim,
  color,
  className = "",
}: {
  prompt: string;
  text: string;
  dim?: boolean;
  color?: string;
  className?: string;
}) {
  return (
    <div className={`flex gap-3 ${className}`}>
      {prompt && (
        <span className="shrink-0 select-none text-accent">{prompt}</span>
      )}
      <span className={color ?? (dim ? "text-muted/50" : "text-heading")}>
        {text}
      </span>
    </div>
  );
}

function OrchestrationDiagram() {
  return (
    <div className="flex h-full flex-col">
      <h3 className="mb-2 text-lg font-bold text-white">
        Multi-agent orchestration
      </h3>
      <p className="mb-8 text-sm text-muted/60">
        Define teams declaratively. Mika handles decomposition, parallel
        execution, quality review, and delivery.
      </p>

      {/* Visual flow */}
      <div className="relative flex flex-1 flex-col items-center justify-center gap-3">
        {/* Goal */}
        <FlowNode label="Goal" sublabel='"Analyze Q3 metrics"' accent />

        <FlowArrow />

        {/* Orchestrator */}
        <FlowNode label="Orchestrator" sublabel="planner" />

        <FlowArrow split />

        {/* Parallel agents */}
        <div className="flex w-full max-w-xs justify-between gap-3">
          <FlowNode label="Specialist" sublabel="researcher" small />
          <FlowNode label="Specialist" sublabel="writer" small />
        </div>

        <FlowArrow merge />

        {/* Critic */}
        <FlowNode label="Critic" sublabel="reviewer" />

        <FlowArrow />

        {/* Deliverable */}
        <FlowNode label="Deliverable" sublabel="deliverable.md" accent />
      </div>
    </div>
  );
}

function FlowNode({
  label,
  sublabel,
  accent,
  small,
}: {
  label: string;
  sublabel: string;
  accent?: boolean;
  small?: boolean;
}) {
  return (
    <div
      className={`flex flex-col items-center justify-center rounded-xl border text-center ${
        small ? "px-4 py-2.5" : "px-6 py-3"
      } ${
        accent
          ? "border-accent/30 bg-accent/10"
          : "border-white/[0.08] bg-white/[0.03]"
      }`}
    >
      <span
        className={`font-semibold ${small ? "text-xs" : "text-sm"} ${
          accent ? "text-accent" : "text-white"
        }`}
      >
        {label}
      </span>
      <span
        className={`font-mono ${small ? "text-[10px]" : "text-xs"} text-muted/50`}
      >
        {sublabel}
      </span>
    </div>
  );
}

function FlowArrow({ split, merge }: { split?: boolean; merge?: boolean }) {
  if (split) {
    return (
      <div className="flex w-full max-w-xs items-start justify-between px-8">
        <svg className="h-6 w-6 text-accent/40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M12 2 L4 10" strokeLinecap="round" />
          <path d="M4 7 L4 10 L7 10" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <svg className="h-6 w-6 text-accent/40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M12 2 L20 10" strokeLinecap="round" />
          <path d="M17 10 L20 10 L20 7" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>
    );
  }

  if (merge) {
    return (
      <div className="flex w-full max-w-xs items-end justify-between px-8">
        <svg className="h-6 w-6 text-accent/40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M4 2 L12 10" strokeLinecap="round" />
          <path d="M9 10 L12 10 L12 7" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
        <svg className="h-6 w-6 text-accent/40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M20 2 L12 10" strokeLinecap="round" />
          <path d="M12 7 L12 10 L15 10" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>
    );
  }

  return (
    <svg className="h-5 w-3 text-accent/40" viewBox="0 0 12 20" fill="none" stroke="currentColor" strokeWidth="2">
      <path d="M6 0 L6 16" strokeLinecap="round" />
      <path d="M2 12 L6 16 L10 12" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

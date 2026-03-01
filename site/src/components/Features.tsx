import {
  Brain,
  Bell,
  Wrench,
  Link,
  MessageCircle,
  ArrowLeftRight,
  Zap,
  TerminalSquare,
} from "lucide-react";
import { FadeIn } from "./FadeIn";
import type { LucideIcon } from "lucide-react";

interface Feature {
  icon: LucideIcon;
  title: string;
  description: string;
}

const features: Feature[] = [
  {
    icon: Brain,
    title: "Persistent Memory",
    description:
      "Remembers people, commitments, and preferences across every conversation.",
  },
  {
    icon: Bell,
    title: "Proactive Reminders",
    description:
      "Schedules reminders, flags conflicts, and surfaces patterns you'd miss.",
  },
  {
    icon: Wrench,
    title: "Skills System",
    description:
      "Extend with custom skills — inject context, keywords, and behaviors.",
  },
  {
    icon: Link,
    title: "MCP Support",
    description:
      "Connect any Model Context Protocol server. Tools appear natively.",
  },
  {
    icon: MessageCircle,
    title: "Telegram Integration",
    description:
      "Approval flows, nudges, and escalations wherever you are.",
  },
  {
    icon: ArrowLeftRight,
    title: "Gateway Router",
    description:
      "Stateless router enables community adapters. Zero business logic.",
  },
  {
    icon: Zap,
    title: "Built in Rust",
    description: "Fast, reliable, minimal footprint. Runs on a $5 VPS.",
  },
  {
    icon: TerminalSquare,
    title: "CLI-First",
    description:
      "Full terminal interface. Works over SSH. No browser required.",
  },
];

export function Features() {
  return (
    <section id="features" className="mx-auto max-w-7xl px-6 py-32 lg:py-40">
      <FadeIn className="mb-16 text-center">
        <h2 className="text-3xl font-extrabold tracking-tight text-white sm:text-4xl lg:text-5xl">
          Everything you need. Nothing you don't.
        </h2>
        <p className="mt-5 text-lg text-muted">
          Mika ships with powerful defaults and gets out of your way.
        </p>
      </FadeIn>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {features.map((feature, i) => (
          <FadeIn key={feature.title} delay={i * 80}>
            <FeatureCard {...feature} />
          </FadeIn>
        ))}
      </div>
    </section>
  );
}

function FeatureCard({ icon: Icon, title, description }: Feature) {
  return (
    <div className="group h-full rounded-2xl border border-white/[0.05] bg-bg-card p-7 transition-all duration-300 hover:border-accent/40 hover:shadow-[0_0_30px_rgba(124,106,247,0.08),inset_0_1px_0_rgba(124,106,247,0.1)]">
      <div className="mb-5 flex h-11 w-11 items-center justify-center rounded-xl bg-accent/10 text-accent transition-colors group-hover:bg-accent group-hover:text-white">
        <Icon className="h-5 w-5" strokeWidth={2} />
      </div>
      <h3 className="text-[15px] font-bold text-white">{title}</h3>
      <p className="mt-2.5 text-[13px] leading-relaxed text-muted/70">
        {description}
      </p>
    </div>
  );
}

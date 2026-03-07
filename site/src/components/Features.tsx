import {
  Brain,
  Bell,
  Wrench,
  Link,
  Globe,
  ListChecks,
  Activity,
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
    title: "Three-Layer Memory",
    description:
      "Core context always in mind. Structured facts for people and commitments. Hybrid search with vector embeddings.",
  },
  {
    icon: Bell,
    title: "Proactive Agent",
    description:
      "Unified task engine with reminders, hourly check-ins, and daily reflection. Mika acts between conversations.",
  },
  {
    icon: Wrench,
    title: "Skills Marketplace",
    description:
      "Install community skills with mika skills install user/repo. Or build your own with a TOML manifest.",
  },
  {
    icon: Link,
    title: "MCP Support",
    description:
      "Connect any Model Context Protocol server. Tools appear natively.",
  },
  {
    icon: Globe,
    title: "Works Everywhere",
    description:
      "Terminal, Telegram, WhatsApp. Your assistant meets you where you are.",
  },
  {
    icon: ListChecks,
    title: "Long-Running Tasks",
    description:
      "Delegate heavy work and get results when ready. Mika tracks progress, handles callbacks, and resumes where it left off.",
  },
  {
    icon: Activity,
    title: "Observability",
    description:
      "OpenTelemetry traces on every agent step. Langfuse compatible. Debug with confidence.",
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

      <div className="flex flex-wrap justify-center gap-4 [&>*]:w-full [&>*]:sm:w-[calc(50%-0.5rem)] [&>*]:lg:w-[calc(25%-0.75rem)]">
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

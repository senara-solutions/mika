import { Download, Settings, Terminal } from "lucide-react";
import { FadeIn } from "./FadeIn";
import type { LucideIcon } from "lucide-react";

interface Step {
  num: number;
  icon: LucideIcon;
  title: string;
  code?: string;
  description: string;
}

const steps: Step[] = [
  {
    num: 1,
    icon: Download,
    title: "Install",
    code: "cargo install mika",
    description: "or pull the Docker image.",
  },
  {
    num: 2,
    icon: Settings,
    title: "Configure",
    description: "Add your API key. Set your preferences.",
  },
  {
    num: 3,
    icon: Terminal,
    title: "Talk to Mika",
    description: "Start chatting. She remembers from here.",
  },
];

export function HowItWorks() {
  return (
    <section
      id="how-it-works"
      className="relative mx-auto max-w-7xl px-6 py-32 lg:py-40"
    >
      <FadeIn className="mb-20 text-center">
        <h2 className="text-3xl font-extrabold tracking-tight text-white sm:text-4xl lg:text-5xl">
          Up and running in minutes.
        </h2>
        <p className="mt-5 text-lg text-muted">
          Three steps. No infrastructure required.
        </p>
      </FadeIn>

      <div className="relative flex flex-col items-start gap-16 md:flex-row md:items-start md:gap-8">
        {/* Connecting line — desktop only */}
        <div
          className="absolute top-8 left-[16.67%] right-[16.67%] hidden h-px md:block"
          style={{
            background:
              "linear-gradient(90deg, transparent, rgba(124,106,247,0.25), transparent)",
          }}
        />

        {steps.map((step, i) => (
          <FadeIn
            key={step.num}
            delay={i * 150}
            className="relative flex flex-1 flex-col items-center text-center"
          >
            <div className="relative z-10 mb-7 flex h-16 w-16 items-center justify-center rounded-full bg-accent text-2xl font-black text-white ring-8 ring-bg shadow-lg shadow-accent/20">
              {step.num}
            </div>
            <h3 className="text-xl font-bold text-white">{step.title}</h3>
            <p className="mt-3 max-w-[240px] text-sm leading-relaxed text-muted/70">
              {step.code && (
                <>
                  <code className="rounded-md bg-white/[0.05] px-2 py-0.5 text-xs font-medium text-accent">
                    {step.code}
                  </code>{" "}
                </>
              )}
              {step.description}
            </p>
          </FadeIn>
        ))}
      </div>
    </section>
  );
}

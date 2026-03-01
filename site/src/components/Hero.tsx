import { useEffect, useState } from "react";

const GITHUB_URL = "https://github.com/senara-solutions/mika";
const DOCS_URL = `${GITHUB_URL}/tree/main/docs`;
const GETTING_STARTED_URL = `${GITHUB_URL}/blob/main/docs/getting-started.md`;

export function Hero() {
  return (
    <header className="relative flex min-h-[92vh] flex-col items-center justify-center overflow-hidden px-6 pt-16 text-center">
      {/* Radial glow */}
      <div
        className="pointer-events-none absolute inset-0 -z-10"
        style={{
          background:
            "radial-gradient(ellipse 800px 500px at 50% 40%, rgba(124,106,247,0.12) 0%, transparent 70%)",
        }}
      />

      {/* Badge */}
      <div className="mb-8 inline-flex items-center gap-2.5 rounded-full border border-accent/20 bg-accent/5 px-5 py-2 text-xs font-semibold uppercase tracking-widest text-accent">
        <span className="relative flex h-2 w-2">
          <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-accent opacity-60" />
          <span className="relative inline-flex h-2 w-2 rounded-full bg-accent" />
        </span>
        Open Source &bull; Built in Rust
      </div>

      {/* Headline */}
      <h1 className="max-w-4xl text-5xl font-black leading-[1.1] tracking-tight text-white sm:text-6xl lg:text-7xl">
        Your AI assistant.
        <br />
        On your terms.
      </h1>

      {/* Subheading */}
      <p className="mt-8 max-w-2xl text-lg leading-relaxed text-muted sm:text-xl">
        Mika is an open-source executive assistant that remembers everything,
        acts proactively, and runs entirely on your machine.
      </p>

      {/* CTAs */}
      <div className="mt-10 flex flex-wrap items-center justify-center gap-4">
        <a
          href={GETTING_STARTED_URL}
          className="flex h-12 items-center justify-center rounded-xl bg-accent px-8 font-semibold text-white shadow-lg shadow-accent/20 transition-all hover:-translate-y-0.5 hover:shadow-accent/30"
        >
          Get Started
        </a>
        <a
          href={DOCS_URL}
          className="flex h-12 items-center justify-center rounded-xl border border-white/10 bg-white/[0.03] px-8 font-semibold text-white transition-all hover:bg-white/[0.07]"
        >
          Read the Docs
        </a>
      </div>

      {/* Terminal Mockup */}
      <TerminalMockup />
    </header>
  );
}

function TerminalMockup() {
  const [step, setStep] = useState(0);

  useEffect(() => {
    const timers = [
      setTimeout(() => setStep(1), 400),
      setTimeout(() => setStep(2), 1500),
      setTimeout(() => setStep(3), 2800),
      setTimeout(() => setStep(4), 4000),
    ];
    return () => timers.forEach(clearTimeout);
  }, []);

  return (
    <div className="mt-20 w-full max-w-3xl overflow-hidden rounded-2xl border border-white/[0.06] bg-bg-card shadow-2xl shadow-black/40">
      {/* Title bar */}
      <div className="flex items-center gap-2 border-b border-white/[0.04] bg-white/[0.02] px-4 py-3">
        <div className="h-3 w-3 rounded-full bg-[#ff5f57]/60" />
        <div className="h-3 w-3 rounded-full bg-[#febc2e]/60" />
        <div className="h-3 w-3 rounded-full bg-[#28c840]/60" />
        <span className="ml-3 text-[11px] font-medium uppercase tracking-widest text-muted/40">
          mika — terminal
        </span>
      </div>

      {/* Content */}
      <div className="p-6 text-left font-mono text-[13px] leading-7 sm:text-sm sm:leading-8">
        {/* Line 1: $ mika chat */}
        <div className="flex gap-3">
          <span className="select-none text-accent">$</span>
          <span
            className={`text-heading type-reveal ${step >= 1 ? "is-visible" : ""}`}
          >
            mika chat
          </span>
        </div>

        {/* Line 2: user input */}
        <div
          className="mt-4 flex gap-3 transition-opacity duration-500"
          style={{ opacity: step >= 2 ? 1 : 0 }}
        >
          <span className="select-none text-muted/50">&gt;</span>
          <span className="text-muted/70">
            Remind me to follow up with Sarah about the Q3 proposal on Monday
          </span>
        </div>

        {/* Line 3: Mika response */}
        <div
          className="mt-4 flex gap-3 transition-opacity duration-600"
          style={{ opacity: step >= 3 ? 1 : 0 }}
        >
          <span className="select-none text-emerald-400">Mika:</span>
          <span className="text-muted/80">
            Done. I've set a reminder for Monday 9am. By the way — you mentioned
            Sarah prefers async updates. Want me to draft an email instead?
          </span>
        </div>

        {/* Line 4: blinking cursor */}
        <div
          className="mt-4 flex gap-3 transition-opacity duration-400"
          style={{ opacity: step >= 4 ? 1 : 0 }}
        >
          <span className="select-none text-accent">$</span>
          <span className="cursor-blink text-heading">_</span>
        </div>
      </div>
    </div>
  );
}

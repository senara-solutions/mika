import { Button } from "@samidarko/ui";

const GITHUB_URL = "https://github.com/senara-solutions/mika";
const DOCS_URL = `${GITHUB_URL}/tree/main/docs`;
const GETTING_STARTED_URL = `${GITHUB_URL}/blob/main/docs/getting-started.md`;

export function Nav() {
  return (
    <nav className="sticky top-0 z-50 border-b border-white/5 bg-bg/80 backdrop-blur-xl">
      <div className="mx-auto flex max-w-7xl items-center justify-between px-6 py-4">
        <a href="#" className="flex items-center gap-1.5">
          <span className="text-xl font-extrabold tracking-tight text-white">
            Mika
          </span>
          <span className="inline-block h-1.5 w-1.5 rounded-full bg-accent" />
        </a>

        <div className="hidden items-center gap-8 md:flex">
          {[
            { label: "Features", href: "#features" },
            { label: "Teams", href: "#teams" },
            { label: "How It Works", href: "#how-it-works" },
            { label: "Docs", href: DOCS_URL },
            { label: "GitHub", href: GITHUB_URL },
          ].map(({ label, href }) => (
            <a
              key={label}
              href={href}
              className="text-sm font-medium text-muted/70 transition-colors hover:text-white"
            >
              {label}
            </a>
          ))}
        </div>

        {/* Was `rounded-full`, which §6 does not offer; the §6 `xl` radius
            (1.5rem) exceeds half this button's height, so it still renders as a
            pill — the shape is preserved without the off-scale class. The glow
            is a landing flourish and stays on `className`. */}
        <Button
          as="link"
          href={GETTING_STARTED_URL}
          className="transition-all hover:shadow-[0_0_24px_color-mix(in_srgb,var(--color-accent)_30%,transparent)]"
        >
          Get Started
        </Button>
      </div>
    </nav>
  );
}

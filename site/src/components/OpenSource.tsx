import { Button } from "@samidarko/ui";
import { FadeIn } from "./FadeIn";

const GITHUB_URL = "https://github.com/senara-solutions/mika";

export function OpenSource() {
  return (
    <section className="mx-auto max-w-7xl px-6 py-20">
      <FadeIn>
        <div
          className="relative overflow-hidden rounded-3xl border border-white/[0.05] p-14 text-center sm:p-20"
          style={{
            background:
              "linear-gradient(135deg, color-mix(in srgb, var(--color-accent) 8%, transparent) 0%, color-mix(in srgb, var(--color-accent) 2%, transparent) 50%, color-mix(in srgb, var(--color-accent) 6%, transparent) 100%)",
          }}
        >
          {/* Glow accents */}
          <div className="pointer-events-none absolute -right-16 -top-16 h-56 w-56 rounded-full bg-accent/15 blur-[80px]" />
          <div className="pointer-events-none absolute -bottom-16 -left-16 h-56 w-56 rounded-full bg-accent/10 blur-[80px]" />

          <div className="relative z-10">
            <h2 className="text-3xl font-black text-white sm:text-4xl lg:text-5xl">
              MIT Licensed. Self-hosted. Yours.
            </h2>
            <p className="mx-auto mt-6 max-w-lg text-lg leading-relaxed text-muted">
              Mika is free and open source. Your data never leaves your machine.
            </p>
            <div className="mt-10 flex flex-wrap items-center justify-center gap-4">
              {/* Was a solid `bg-white` fill with `text-bg` — a §7 "No Pure
                  White" violation, and a fourth CTA treatment that §5 does not
                  offer. Same arbitration as SkillVariants' green Promote: §8
                  reserves a new variant to Vincent, so this becomes `primary`
                  and loses its white. */}
              <Button
                as="link"
                href={GITHUB_URL}
                size="lg"
                className="hover:shadow-lg"
                icon={
                  <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
                  </svg>
                }
              >
                Star on GitHub
              </Button>
              {/* Byte-for-byte the same class string as Hero's secondary, which
                  the plan's R2 table did catalogue. Both migrate to the §5 ghost. */}
              <Button as="link" href={GITHUB_URL} variant="secondary" size="lg">
                View Source
              </Button>
            </div>
          </div>
        </div>
      </FadeIn>
    </section>
  );
}

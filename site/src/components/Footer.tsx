const GITHUB_URL = "https://github.com/senara-solutions/mika";
const DOCS_URL = `${GITHUB_URL}/tree/main/docs`;
const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

export function Footer() {
  return (
    <footer className="mt-16 border-t border-white/[0.04]">
      <div className="mx-auto flex max-w-7xl flex-col items-center justify-between gap-6 px-6 py-12 sm:flex-row">
        <div className="flex items-center gap-4">
          <a href="#" className="flex items-center gap-1.5">
            <span className="text-lg font-extrabold tracking-tight text-white">
              Mika
            </span>
            <span className="inline-block h-1.5 w-1.5 rounded-full bg-accent" />
          </a>
          <span className="text-sm text-muted/40">&copy; {new Date().getFullYear()}</span>
        </div>

        <div className="flex items-center gap-8">
          <a
            href={GITHUB_URL}
            className="text-sm text-muted/50 transition-colors hover:text-white"
          >
            GitHub
          </a>
          <a
            href={DOCS_URL}
            className="text-sm text-muted/50 transition-colors hover:text-white"
          >
            Documentation
          </a>
          <a
            href={LICENSE_URL}
            className="text-sm text-muted/50 transition-colors hover:text-white"
          >
            MIT License
          </a>
        </div>

        <p className="text-sm text-muted/30">
          Backed by Senara Solutions &{" "}
          <a
            href="https://superuserhq.com/"
            className="text-muted/50 transition-colors hover:text-white"
          >
            Superuser HQ
          </a>
        </p>
      </div>
    </footer>
  );
}

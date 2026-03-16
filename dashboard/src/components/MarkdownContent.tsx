import ReactMarkdown from 'react-markdown'

export default function MarkdownContent({
  content,
  className = '',
}: {
  content: string
  className?: string
}) {
  return (
    <div className={`text-sm text-muted space-y-3 ${className}`}>
      <ReactMarkdown
        components={{
          h1: ({ children }) => (
            <h1 className="text-heading text-lg font-semibold mt-4 mb-2">{children}</h1>
          ),
          h2: ({ children }) => (
            <h2 className="text-heading text-base font-semibold mt-3 mb-2">{children}</h2>
          ),
          h3: ({ children }) => (
            <h3 className="text-heading text-sm font-semibold mt-3 mb-1">{children}</h3>
          ),
          h4: ({ children }) => (
            <h4 className="text-heading text-sm font-medium mt-2 mb-1">{children}</h4>
          ),
          p: ({ children }) => <p className="leading-relaxed">{children}</p>,
          ul: ({ children }) => <ul className="list-disc list-inside space-y-1">{children}</ul>,
          ol: ({ children }) => (
            <ol className="list-decimal list-inside space-y-1">{children}</ol>
          ),
          li: ({ children }) => <li className="leading-relaxed">{children}</li>,
          a: ({ href, children }) => (
            <a
              href={href}
              className="text-accent hover:text-accent-light underline"
              target="_blank"
              rel="noopener noreferrer"
            >
              {children}
            </a>
          ),
          code: ({ children, className }) => {
            const isBlock = className?.includes('language-')
            if (isBlock) {
              return (
                <code className="block bg-white/[0.04] border border-white/[0.06] rounded-lg px-3 py-2 text-xs font-mono text-muted/90 overflow-x-auto">
                  {children}
                </code>
              )
            }
            return (
              <code className="bg-white/[0.06] px-1 py-0.5 rounded text-xs font-mono text-heading">
                {children}
              </code>
            )
          },
          pre: ({ children }) => <div className="my-2">{children}</div>,
          blockquote: ({ children }) => (
            <blockquote className="border-l-2 border-accent/30 pl-3 text-muted/70 italic">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="border-white/[0.06] my-3" />,
          strong: ({ children }) => (
            <strong className="text-heading font-semibold">{children}</strong>
          ),
          table: ({ children }) => (
            <div className="overflow-x-auto">
              <table className="w-full text-xs border-collapse">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="text-left px-2 py-1 border-b border-white/[0.08] text-heading font-medium">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="px-2 py-1 border-b border-white/[0.04]">{children}</td>
          ),
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  )
}

import { Link } from 'react-router'

export default function NotFound() {
  return (
    <div className="flex flex-col items-center justify-center py-24 text-muted/60">
      <h2 className="text-heading text-2xl font-semibold mb-2">404</h2>
      <p className="text-sm mb-4">Page not found</p>
      <Link
        to="/"
        className="text-sm text-accent hover:text-accent-light transition-colors"
      >
        Back to Timeline
      </Link>
    </div>
  )
}

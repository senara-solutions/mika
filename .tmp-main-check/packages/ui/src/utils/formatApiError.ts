/**
 * Convert a raw error object to a human-shaped error message.
 *
 * Canonical conversion path for `<ErrorState message={formatApiError(error)} />`.
 * Four cases:
 *   1. Network error (TypeError from fetch) → connectivity message
 *   2. Server error with `detail` field (Axum/FastAPI envelope) → detail text
 *   3. Error instance fallback → error.message
 *   4. Unknown shape → generic message
 *
 * React Query v4 types error as `Error | null`; v5 types it as `unknown`.
 * The `instanceof Error` guard keeps this correct across both versions.
 */
export function formatApiError(error: unknown): string {
  if (error instanceof TypeError && error.message.includes('fetch')) {
    return 'Network unreachable. Check your connection.'
  }
  if (
    typeof error === 'object' &&
    error !== null &&
    'detail' in error &&
    typeof (error as { detail: unknown }).detail === 'string'
  ) {
    return (error as { detail: string }).detail
  }
  if (error instanceof Error) {
    return error.message
  }
  return 'An unexpected error occurred.'
}

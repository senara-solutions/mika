/**
 * Parse a GitHub URL into its components.
 *
 * Supports:
 * - https://github.com/owner/repo/issues/123
 * - https://github.com/owner/repo/pull/123
 */
export interface GitHubRef {
  owner: string
  repo: string
  number: number
  type: 'issue' | 'pull'
}

const GITHUB_URL_RE = /^https?:\/\/github\.com\/([^/]+)\/([^/]+)\/(issues|pull)\/(\d+)/

export function parseGitHubUrl(url: string | null | undefined): GitHubRef | null {
  if (!url) return null
  const match = url.match(GITHUB_URL_RE)
  if (!match) return null
  return {
    owner: match[1],
    repo: match[2],
    number: parseInt(match[3] === 'issues' ? match[4] : match[4], 10),
    type: match[3] === 'issues' ? 'issue' : 'pull',
  }
}

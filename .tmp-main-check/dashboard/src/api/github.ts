import { useQuery } from '@tanstack/react-query'
import { apiFetch } from './client.ts'

export interface GitHubIssue {
  title: string
  body: string | null
  state: string
  labels: Array<{ name: string; color: string }>
}

export interface GitHubReview {
  user: string
  state: string
  body: string | null
  submitted_at: string | null
}

export interface GitHubPull {
  title: string
  body: string | null
  state: string
  additions: number
  deletions: number
  changed_files: number
  merged: boolean
  reviews: GitHubReview[]
}

export function useGitHubIssue(
  owner: string | null,
  repo: string | null,
  number: number | null,
) {
  return useQuery<GitHubIssue>({
    queryKey: ['github-issue', owner, repo, number],
    queryFn: () => apiFetch(`/github/issues/${owner}/${repo}/${number}`),
    enabled: !!owner && !!repo && !!number,
    staleTime: 5 * 60 * 1000, // 5 min — issue body doesn't change often
    retry: 1,
  })
}

export function useGitHubPull(
  owner: string | null,
  repo: string | null,
  number: number | null,
) {
  return useQuery<GitHubPull>({
    queryKey: ['github-pull', owner, repo, number],
    queryFn: () => apiFetch(`/github/pulls/${owner}/${repo}/${number}`),
    enabled: !!owner && !!repo && !!number,
    staleTime: 2 * 60 * 1000, // 2 min — PR stats change more often
    retry: 1,
  })
}

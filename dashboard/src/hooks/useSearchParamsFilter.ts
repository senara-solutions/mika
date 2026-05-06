import { useSearchParams } from 'react-router'

/** All page-related URL params. Filter changes clear all of these to reset pagination. */
export const ALL_PAGE_PARAMS = ['page', 'wi_page', 'trt_page', 'cb_page', 'sched_page'] as const

export function useSearchParamsFilter() {
  const [searchParams, setSearchParams] = useSearchParams()

  function updateFilter(key: string, value: string) {
    const next = new URLSearchParams(searchParams)
    if (value) {
      next.set(key, value)
    } else {
      next.delete(key)
    }
    ALL_PAGE_PARAMS.forEach((k) => next.delete(k))
    setSearchParams(next)
  }

  function setPage(page: number) {
    const next = new URLSearchParams(searchParams)
    next.set('page', String(page))
    setSearchParams(next)
  }

  function setSectionPage(key: string, page: number) {
    const next = new URLSearchParams(searchParams)
    next.set(key, String(page))
    setSearchParams(next)
  }

  return { searchParams, setSearchParams, updateFilter, setPage, setSectionPage }
}

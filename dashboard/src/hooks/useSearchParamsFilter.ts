import { useSearchParams } from 'react-router'

export function useSearchParamsFilter() {
  const [searchParams, setSearchParams] = useSearchParams()

  function updateFilter(key: string, value: string) {
    const next = new URLSearchParams(searchParams)
    if (value) {
      next.set(key, value)
    } else {
      next.delete(key)
    }
    next.delete('page')
    setSearchParams(next)
  }

  function setPage(page: number) {
    const next = new URLSearchParams(searchParams)
    next.set('page', String(page))
    setSearchParams(next)
  }

  return { searchParams, setSearchParams, updateFilter, setPage }
}

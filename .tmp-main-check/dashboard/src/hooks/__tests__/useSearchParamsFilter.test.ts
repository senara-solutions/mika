import { renderHook, act } from '@testing-library/react'
import { describe, it, expect } from 'vitest'
import { createElement } from 'react'
import { MemoryRouter } from 'react-router'
import { useSearchParamsFilter, ALL_PAGE_PARAMS } from '../useSearchParamsFilter'

function wrapper({ children }: { children: React.ReactNode }) {
  return createElement(MemoryRouter, null, children)
}

function wrapperWithParams(initialEntries: string[]) {
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return createElement(MemoryRouter, { initialEntries }, children)
  }
}

describe('useSearchParamsFilter', () => {
  it('returns searchParams, setSearchParams, updateFilter, setPage, setSectionPage', () => {
    const { result } = renderHook(() => useSearchParamsFilter(), { wrapper })
    expect(result.current.searchParams).toBeDefined()
    expect(result.current.setSearchParams).toBeTypeOf('function')
    expect(result.current.updateFilter).toBeTypeOf('function')
    expect(result.current.setPage).toBeTypeOf('function')
    expect(result.current.setSectionPage).toBeTypeOf('function')
  })

  describe('updateFilter', () => {
    it('sets a filter param and clears all page params', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), {
        wrapper: wrapperWithParams(['/?page=3&wi_page=2&trt_page=4']),
      })
      act(() => result.current.updateFilter('agent_id', 'mika-dev'))
      expect(result.current.searchParams.get('agent_id')).toBe('mika-dev')
      // All page params should be cleared
      for (const key of ALL_PAGE_PARAMS) {
        expect(result.current.searchParams.get(key)).toBeNull()
      }
    })

    it('deletes a filter param when value is empty', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), {
        wrapper: wrapperWithParams(['/?agent_id=mika-dev']),
      })
      act(() => result.current.updateFilter('agent_id', ''))
      expect(result.current.searchParams.get('agent_id')).toBeNull()
    })

    it('does not error when no page params exist', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), { wrapper })
      act(() => result.current.updateFilter('agent_id', 'mika-dev'))
      expect(result.current.searchParams.get('agent_id')).toBe('mika-dev')
    })

    it('preserves non-page params when clearing pages', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), {
        wrapper: wrapperWithParams(['/?from=2026-01-01&page=2&wi_page=3']),
      })
      act(() => result.current.updateFilter('agent_id', 'mika-dev'))
      expect(result.current.searchParams.get('from')).toBe('2026-01-01')
      expect(result.current.searchParams.get('agent_id')).toBe('mika-dev')
      expect(result.current.searchParams.get('page')).toBeNull()
      expect(result.current.searchParams.get('wi_page')).toBeNull()
    })
  })

  describe('setPage', () => {
    it('sets the page param', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), { wrapper })
      act(() => result.current.setPage(5))
      expect(result.current.searchParams.get('page')).toBe('5')
    })
  })

  describe('setSectionPage', () => {
    it('sets a section page param without clearing other params', () => {
      const { result } = renderHook(() => useSearchParamsFilter(), {
        wrapper: wrapperWithParams(['/?page=2&from=2026-01-01']),
      })
      act(() => result.current.setSectionPage('wi_page', 3))
      expect(result.current.searchParams.get('wi_page')).toBe('3')
      // Other params preserved
      expect(result.current.searchParams.get('page')).toBe('2')
      expect(result.current.searchParams.get('from')).toBe('2026-01-01')
    })
  })

  describe('ALL_PAGE_PARAMS', () => {
    it('exports the expected page param keys', () => {
      expect(ALL_PAGE_PARAMS).toContain('page')
      expect(ALL_PAGE_PARAMS).toContain('wi_page')
      expect(ALL_PAGE_PARAMS).toContain('trt_page')
      expect(ALL_PAGE_PARAMS).toContain('cb_page')
      expect(ALL_PAGE_PARAMS).toContain('sched_page')
      expect(ALL_PAGE_PARAMS).toHaveLength(5)
    })
  })
})

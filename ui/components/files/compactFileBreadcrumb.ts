export interface BreadcrumbPathSegment {
  id: string
  name: string
}

export interface IndexedBreadcrumbSegment<T extends BreadcrumbPathSegment = BreadcrumbPathSegment> {
  index: number
  segment: T
}

export interface CompactBreadcrumbLayout<T extends BreadcrumbPathSegment = BreadcrumbPathSegment> {
  collapsed: boolean
  hidden: IndexedBreadcrumbSegment<T>[]
  visible: IndexedBreadcrumbSegment<T>[]
}

export interface BreadcrumbNavigationTarget {
  id: string
  index: number
}

export const COMPACT_BREADCRUMB_MAX_LEVELS = 4
const COMPACT_BREADCRUMB_TAIL_LEVELS = 3

function withIndex<T extends BreadcrumbPathSegment>(segment: T, index: number): IndexedBreadcrumbSegment<T> {
  return { index, segment }
}

export function buildCompactBreadcrumbLayout<T extends BreadcrumbPathSegment>(
  segments: readonly T[],
): CompactBreadcrumbLayout<T> {
  if (segments.length <= COMPACT_BREADCRUMB_MAX_LEVELS) {
    return {
      collapsed: false,
      hidden: [],
      visible: segments.map(withIndex),
    }
  }

  const tailStart = segments.length - COMPACT_BREADCRUMB_TAIL_LEVELS

  return {
    collapsed: true,
    hidden: segments.slice(1, tailStart).map((segment, offset) => withIndex(segment, offset + 1)),
    visible: [
      withIndex(segments[0], 0),
      ...segments.slice(tailStart).map((segment, offset) => withIndex(segment, tailStart + offset)),
    ],
  }
}

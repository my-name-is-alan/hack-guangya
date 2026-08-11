const NODE_MODULES_MARKER = '/node_modules/'

// These directories form one strongly connected component inside antdv-next.
// Keeping the cycle together lets Rollup split the remaining leaf components
// without creating circular chunks.
const ANTDV_CORE_SEGMENTS = new Set([
  '_util',
  'button',
  'checkbox',
  'collapse',
  'color-picker',
  'config-provider',
  'dropdown',
  'empty',
  'form',
  'grid',
  'input',
  'layout',
  'locale',
  'menu',
  'pagination',
  'radio',
  'select',
  'space',
  'spin',
  'style',
  'table',
  'theme',
  'time-picker',
  'tooltip',
  'tree',
  'version',
  'calendar',
  'date-picker',
  'package.js',
])

/**
 * Return the package that owns a resolved Vite module id.
 *
 * pnpm stores packages under a virtual-store path that can contain the names
 * of peer dependencies. Looking only at the final node_modules segment keeps
 * chunk names stable when dependency versions or peer suffixes change.
 */
export function packageNameFromModuleId(moduleId) {
  const normalizedId = moduleId.replaceAll('\\', '/')
  const markerIndex = normalizedId.lastIndexOf(NODE_MODULES_MARKER)
  if (markerIndex < 0) return null

  const packagePath = normalizedId.slice(markerIndex + NODE_MODULES_MARKER.length)
  const parts = packagePath.split('/')
  if (!parts[0]) return null
  return parts[0].startsWith('@') && parts[1]
    ? `${parts[0]}/${parts[1]}`
    : parts[0]
}

export function antdvChunkName(moduleId) {
  const normalizedId = moduleId.replaceAll('\\', '/')
  const marker = '/node_modules/antdv-next/dist/'
  const markerIndex = normalizedId.lastIndexOf(marker)
  if (markerIndex < 0) return undefined

  const segment = normalizedId.slice(markerIndex + marker.length).split('/')[0]
  return ANTDV_CORE_SEGMENTS.has(segment)
    ? 'vendor-antdv-core'
    : 'vendor-antdv-components'
}

/**
 * Keep large, long-lived framework dependencies out of the application entry
 * without producing a separate request for every npm package.
 */
export function vendorChunkName(moduleId) {
  const packageName = packageNameFromModuleId(moduleId)
  if (!packageName) return undefined

  if (packageName === 'antdv-next') return antdvChunkName(moduleId)
  // antdv-style imports the public antdv-next entry. It must stay with that
  // entry instead of the lower-level runtime packages to keep chunks acyclic.
  if (packageName === 'antdv-style') return 'vendor-antdv-components'
  if (packageName === '@antdv-next/icons') return 'vendor-icons'
  // cssinjs consumes @v-c/util directly, so it belongs on the same side of
  // the dependency boundary as the v-c component runtimes.
  if (packageName === '@antdv-next/cssinjs') return 'vendor-vc'
  if (packageName.startsWith('@v-c/')) return 'vendor-vc'
  if (
    packageName === 'vue'
    || packageName === 'vue-router'
    || packageName === 'pinia'
    || packageName.startsWith('@vue/')
  ) {
    return 'vendor-vue'
  }
  return 'vendor-runtime'
}

let activeClear = null
const listeners = new Set()

function notify(active) {
  for (const listener of listeners) {
    try { listener(active) }
    catch { /* UI subscribers cannot change the operation result. */ }
  }
}

export function subscribeRecycleBinClear(listener) {
  if (typeof listener !== 'function') return () => {}
  listeners.add(listener)
  try { listener(Boolean(activeClear)) }
  catch { /* A view can disappear while its subscription is being installed. */ }
  return () => listeners.delete(listener)
}

export function requestRecycleBinClear(run) {
  if (activeClear) return { started: false, promise: activeClear }

  const operation = Promise.resolve().then(run)
  const settled = operation.finally(() => {
    if (activeClear !== settled) return
    activeClear = null
    notify(false)
  })
  activeClear = settled
  notify(true)
  return { started: true, promise: settled }
}

export async function waitForRecycleBinClear() {
  const pending = activeClear
  if (!pending) return
  try {
    await pending
  }
  catch {
    // A failed clear must not prevent the recycle-bin list from being loaded.
  }
}

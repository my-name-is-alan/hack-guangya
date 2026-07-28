/// <reference types="vite/client" />

interface Window {
  __TAURI__?: {
    core?: { invoke: <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T> }
    event?: { listen: (event: string, callback: (event: { payload: any }) => void) => Promise<() => void> }
  }
}

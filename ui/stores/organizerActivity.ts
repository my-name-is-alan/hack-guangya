import { computed, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { bridge } from '../bridge.js'
import { unwrapData } from '../formatters.js'

/**
 * 整理任务的全局活动指示：识别中 / 整理中数量显示在顶栏，
 * 由 organizer 事件驱动刷新（节流），供任何页面感知整理进度。
 */
export const useOrganizerActivityStore = defineStore('organizerActivity', () => {
  const recognizing = shallowRef(0)
  const running = shallowRef(0)
  const ready = shallowRef(0)
  const needsReview = shallowRef(0)
  const active = computed(() => recognizing.value + running.value)
  let unsubscribe: null | (() => void) = null
  let refreshTimer: ReturnType<typeof setTimeout> | null = null
  let started = false

  async function refresh() {
    try {
      const data = unwrapData(await bridge.invoke('get_organizer_state')) as { counts?: Record<string, number> }
      const counts = data?.counts || {}
      recognizing.value = Number(counts.recognizing || 0)
      running.value = Number(counts.running || 0)
      ready.value = Number(counts.ready || 0)
      needsReview.value = Number(counts.needs_review || 0) + Number(counts.failed || 0)
    } catch {
      // 未登录/未配置时静默；下一次事件会再尝试
    }
  }

  function scheduleRefresh() {
    if (refreshTimer) return
    refreshTimer = setTimeout(() => {
      refreshTimer = null
      void refresh()
    }, 800)
  }

  async function start() {
    if (started) return
    started = true
    void refresh()
    try {
      unsubscribe = await bridge.subscribe((event: any) => {
        if (event?.type === 'organizer') scheduleRefresh()
      })
    } catch {
      // 订阅失败退化为无实时刷新
    }
  }

  function dispose() {
    unsubscribe?.()
    unsubscribe = null
    if (refreshTimer) clearTimeout(refreshTimer)
    refreshTimer = null
    started = false
  }

  return { recognizing, running, ready, needsReview, active, refresh, start, dispose }
})

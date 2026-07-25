<script setup lang="ts">
import { computed, h, shallowRef } from 'vue'
import { Dropdown as ADropdown } from 'antdv-next'
import type { MenuItemType, MenuProps } from 'antdv-next'
import {
  buildCompactBreadcrumbLayout,
  type BreadcrumbNavigationTarget,
  type BreadcrumbPathSegment,
  type IndexedBreadcrumbSegment,
} from './compactFileBreadcrumb'

const props = defineProps<{
  segments: readonly BreadcrumbPathSegment[]
}>()

const emit = defineEmits<{
  navigate: [target: BreadcrumbNavigationTarget]
}>()

const hiddenOpen = shallowRef(false)
const layout = computed(() => buildCompactBreadcrumbLayout(props.segments))
const currentIndex = computed(() => props.segments.length - 1)

const hiddenMenuItems = computed<MenuItemType[]>(() => layout.value.hidden.map(({ index, segment }) => ({
  key: String(index),
  label: h('span', {
    style: {
      display: 'block',
      maxWidth: '280px',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
      whiteSpace: 'nowrap',
    },
    title: segment.name,
  }, segment.name),
})))

function navigateTo(index: number) {
  const segment = props.segments[index]
  if (!segment || index === currentIndex.value) return

  hiddenOpen.value = false
  emit('navigate', { index, id: segment.id })
}

const handleHiddenMenuClick: NonNullable<MenuProps['onClick']> = ({ key }) => {
  navigateTo(Number(key))
}

function itemKey(item: IndexedBreadcrumbSegment) {
  return `${item.segment.id || 'root'}:${item.index}`
}
</script>

<template>
  <nav v-if="segments.length" class="compact-file-breadcrumb" aria-label="文件夹路径">
    <ol class="compact-file-breadcrumb__list">
      <template v-for="(item, visibleIndex) in layout.visible" :key="itemKey(item)">
        <li v-if="visibleIndex > 0" class="compact-file-breadcrumb__separator" aria-hidden="true">/</li>

        <li v-if="layout.collapsed && visibleIndex === 1" class="compact-file-breadcrumb__ellipsis-item">
          <ADropdown
            v-model:open="hiddenOpen"
            :auto-focus="true"
            :menu="{ items: hiddenMenuItems, onClick: handleHiddenMenuClick }"
            :trigger="['click']"
            placement="bottomLeft"
          >
            <button
              type="button"
              class="compact-file-breadcrumb__ellipsis"
              aria-haspopup="menu"
              :aria-expanded="hiddenOpen"
              :aria-label="`显示已折叠的 ${layout.hidden.length} 层文件夹`"
              title="显示中间路径"
            >…</button>
          </ADropdown>
        </li>

        <li
          v-if="layout.collapsed && visibleIndex === 1"
          class="compact-file-breadcrumb__separator"
          aria-hidden="true"
        >/</li>

        <li
          class="compact-file-breadcrumb__item"
          :class="{ 'compact-file-breadcrumb__item--current': item.index === currentIndex }"
        >
          <span
            v-if="item.index === currentIndex"
            class="compact-file-breadcrumb__node compact-file-breadcrumb__node--current"
            aria-current="page"
            :title="item.segment.name"
          >{{ item.segment.name }}</span>
          <button
            v-else
            type="button"
            class="compact-file-breadcrumb__node compact-file-breadcrumb__node--link"
            :title="item.segment.name"
            @click="navigateTo(item.index)"
          >{{ item.segment.name }}</button>
        </li>
      </template>
    </ol>
  </nav>
</template>

<style scoped>
.compact-file-breadcrumb {
  width: min(40vw, 460px);
  min-width: 0;
  max-width: 100%;
  color: var(--text-3);
}

.compact-file-breadcrumb__list {
  display: flex;
  min-width: 0;
  max-width: 100%;
  align-items: center;
  gap: 2px;
  margin: 0;
  padding: 0;
  overflow: hidden;
  list-style: none;
  white-space: nowrap;
}

.compact-file-breadcrumb__item {
  display: flex;
  min-width: 0;
  max-width: 160px;
  flex: 0 1 auto;
  align-items: center;
}

.compact-file-breadcrumb__item--current {
  flex: 1 1 80px;
}

.compact-file-breadcrumb__node,
.compact-file-breadcrumb__ellipsis {
  display: block;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  border: 0;
  border-radius: var(--r-sm);
  background: transparent;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.compact-file-breadcrumb__node {
  padding: 3px 5px;
}

.compact-file-breadcrumb__node--link,
.compact-file-breadcrumb__ellipsis {
  color: var(--text-2);
  cursor: pointer;
}

.compact-file-breadcrumb__node--link:hover,
.compact-file-breadcrumb__ellipsis:hover,
.compact-file-breadcrumb__ellipsis[aria-expanded='true'] {
  color: var(--primary);
  background: var(--primary-line);
}

.compact-file-breadcrumb__node--current {
  color: var(--text-1);
  font-weight: 600;
}

.compact-file-breadcrumb__separator {
  flex: 0 0 auto;
  color: var(--line-strong);
  user-select: none;
}

.compact-file-breadcrumb__ellipsis-item {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
}

.compact-file-breadcrumb__ellipsis {
  width: 26px;
  height: 26px;
  padding: 0;
  text-align: center;
  letter-spacing: 1px;
}
</style>

<script setup>
// 统一的页面标题：标题 + 一句话说明 + 右侧操作区。
// 字号锁定在 token（--fs-xl / --fs-sm），替代各页面自造的标题排版。
defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' },
});
</script>

<template>
  <header class="page-header">
    <div class="page-header-text">
      <h1>{{ title }}</h1>
      <p v-if="description">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="page-header-actions">
      <slot name="actions" />
    </div>
  </header>
</template>

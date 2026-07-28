<script setup lang="ts">
import { onBeforeUnmount, onMounted } from 'vue'
import { storeToRefs } from 'pinia'
import useIllustrationTheme from './illustrationTheme'
import { useSessionStore } from './stores/session'
import AccessGate from './components/auth/AccessGate.vue'
import AuthGate from './components/auth/AuthGate.vue'
import AppShell from './components/shell/AppShell.vue'

const configProps = useIllustrationTheme()
const session = useSessionStore()
const { bootLoading, accessGranted, forceAuth } = storeToRefs(session)

onMounted(() => session.initialize())
onBeforeUnmount(() => session.dispose())
</script>

<template>
  <a-config-provider v-bind="configProps">
    <a-app>
      <div v-if="bootLoading" class="boot-screen"><a-spin size="large" /><span>正在连接本地服务…</span></div>
      <AccessGate v-else-if="!accessGranted" />
      <AuthGate v-else-if="forceAuth || !session.state.logged_in" />
      <AppShell v-else />
    </a-app>
  </a-config-provider>
</template>

<style scoped>
.boot-screen { display: flex; min-height: 100vh; align-items: center; flex-direction: column; justify-content: center; gap: 16px; color: var(--text-3, #98a2b3); background: var(--app-bg, #f7f7f8); }
</style>

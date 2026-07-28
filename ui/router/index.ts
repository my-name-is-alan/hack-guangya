import { createRouter, createWebHashHistory } from 'vue-router'

const routes = [
  { path: '/', redirect: '/files' },
  { path: '/files', name: 'files', component: () => import('../views/CloudView.vue') },
  { path: '/backup', name: 'backup', component: () => import('../views/BackupView.vue') },
  { path: '/transfers', name: 'transfers', component: () => import('../views/TransfersView.vue') },
  { path: '/offline', name: 'offline', component: () => import('../views/OfflineView.vue') },
  { path: '/shares', name: 'shares', component: () => import('../views/SharesView.vue') },
  { path: '/settings', name: 'settings', component: () => import('../views/SystemSettingsView.vue') },
  { path: '/:pathMatch(.*)*', redirect: '/files' },
]

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
})

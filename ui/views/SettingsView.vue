<script setup>
import { reactive, ref } from 'vue';
import { message } from 'antdv-next';
import {
  CloudOutlined,
  ControlOutlined,
  FileTextOutlined,
  LoginOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
  UserOutlined,
} from '@antdv-next/icons';
import { bridge, isTauri } from '../bridge.js';
import {
  appState,
  formatSize,
  formatTime,
  isVip,
  loadOverview,
  profileId,
  profilePhone,
  quotaPercent,
  refreshState,
  totalSpace,
  usedSpace,
  userAvatar,
  userName,
  vipExpireLabel,
  vipLabel,
} from '../store.js';
import { errorText, unwrapData } from '../formatters.js';

const transferSettingsSaving = ref(false);
const transferForm = reactive({
  upload_concurrency: appState.upload_concurrency || 1,
  download_concurrency: appState.download_concurrency || 1,
});

function normalizeTransferConcurrency(value) {
  const number = Number(value);
  if (!Number.isFinite(number)) return 1;
  return Math.min(8, Math.max(1, Math.round(number)));
}

async function saveTransferSettings() {
  transferSettingsSaving.value = true;
  try {
    const next = unwrapData(await bridge.invoke('update_transfer_settings', {
      upload_concurrency: normalizeTransferConcurrency(transferForm.upload_concurrency),
      download_concurrency: normalizeTransferConcurrency(transferForm.download_concurrency),
    }));
    Object.assign(appState, next);
    message.success('传输并发设置已保存');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    transferSettingsSaving.value = false;
  }
}

const emit = defineEmits(['login']);
</script>

<template>
  <div class="view-section settings-view">
    <a-row :gutter="[14, 14]">
      <a-col :xs="24" :lg="12">
        <a-card class="content-card" :bordered="false">
          <template #title><a-flex align="center" gap="small"><UserOutlined />账号信息</a-flex></template>
          <template #extra>
            <a-button size="small" :disabled="!appState.logged_in" @click="loadOverview"><template #icon><ReloadOutlined /></template></a-button>
          </template>
          <a-flex align="center" gap="middle" class="settings-account">
            <a-avatar :size="52" :src="userAvatar || undefined"><template #icon><UserOutlined /></template></a-avatar>
            <div class="settings-account-main">
              <strong>{{ userName }}</strong>
              <a-tag :color="isVip ? 'gold' : 'default'" style="width:fit-content">{{ vipLabel }}</a-tag>
            </div>
            <a-button v-if="!appState.logged_in" type="primary" @click="emit('login')"><template #icon><LoginOutlined /></template>登录云盘</a-button>
          </a-flex>
          <a-descriptions :column="1" size="small" bordered style="margin-top: 14px">
            <a-descriptions-item label="账号 ID">{{ profileId }}</a-descriptions-item>
            <a-descriptions-item label="手机号">{{ profilePhone }}</a-descriptions-item>
            <a-descriptions-item label="VIP 到期">{{ vipExpireLabel }}</a-descriptions-item>
            <a-descriptions-item label="存储空间">
              <template v-if="totalSpace">
                <a-progress :percent="quotaPercent" size="small" style="max-width: 260px" />
                <small>已用 {{ formatSize(usedSpace) }} / {{ formatSize(totalSpace) }}</small>
              </template>
              <span v-else>{{ appState.logged_in ? '读取中…' : '登录后显示' }}</span>
            </a-descriptions-item>
          </a-descriptions>
        </a-card>
      </a-col>

      <a-col :xs="24" :lg="12">
        <a-card class="content-card" :bordered="false">
          <template #title><a-flex align="center" gap="small"><ControlOutlined />传输设置</a-flex></template>
          <a-form layout="vertical">
            <a-form-item label="上传并发数">
              <a-input-number v-model:value="transferForm.upload_concurrency" :min="1" :max="8" style="width: 100%" />
            </a-form-item>
            <a-form-item label="下载并发数">
              <a-input-number v-model:value="transferForm.download_concurrency" :min="1" :max="8" style="width: 100%" />
            </a-form-item>
            <a-button type="primary" :loading="transferSettingsSaving" @click="saveTransferSettings">保存</a-button>
          </a-form>
        </a-card>

        <a-card class="content-card" :bordered="false" style="margin-top: 14px">
          <template #title><a-flex align="center" gap="small"><CloudOutlined />队列控制</a-flex></template>
          <a-flex align="center" justify="space-between" wrap="wrap" gap="small">
            <div>
              <strong>{{ appState.paused ? '队列已暂停' : '队列运行中' }}</strong>
              <p class="settings-desc">{{ appState.active_uploads }} 个上传中 · {{ appState.pending }} 个等待</p>
            </div>
            <a-button v-if="appState.paused" type="primary" @click="bridge.invoke('resume_queue').then(refreshState)">恢复队列</a-button>
            <a-button v-else @click="bridge.invoke('pause_queue').then(refreshState)">暂停队列</a-button>
          </a-flex>
        </a-card>
      </a-col>

      <a-col :xs="24" :lg="12">
        <a-card class="content-card" :bordered="false">
          <template #title><a-flex align="center" gap="small"><SafetyCertificateOutlined />运行环境</a-flex></template>
          <a-descriptions :column="1" size="small" bordered>
            <a-descriptions-item label="运行模式">{{ isTauri ? '桌面端（Tauri）' : 'Docker Web 控制台' }}</a-descriptions-item>
            <a-descriptions-item label="连接状态">{{ appState.logged_in ? '云盘已连接' : '未登录' }}</a-descriptions-item>
            <a-descriptions-item label="令牌存储">仅保存在运行内存</a-descriptions-item>
            <a-descriptions-item label="备份任务">{{ appState.mappings.length }} 个</a-descriptions-item>
          </a-descriptions>
        </a-card>
      </a-col>

      <a-col :xs="24" :lg="12">
        <a-card class="content-card" :bordered="false">
          <template #title><a-flex align="center" gap="small"><FileTextOutlined />运行日志</a-flex></template>
          <template #extra><a-button size="small" @click="refreshState"><template #icon><ReloadOutlined /></template></a-button></template>
          <div class="log-list">
            <a-empty v-if="!appState.logs.length" description="暂无日志" :image-style="{ height: '60px' }" />
            <div v-for="(log, index) in appState.logs.slice(0, 50)" :key="index" class="log-row">
              <a-tag :color="log.level === 'error' ? 'error' : log.level === 'warn' ? 'warning' : 'default'">{{ log.level || 'info' }}</a-tag>
              <span class="log-message" :title="log.message">{{ log.message }}</span>
              <span class="log-time">{{ formatTime(log.time) }}</span>
            </div>
          </div>
        </a-card>
      </a-col>
    </a-row>
  </div>
</template>

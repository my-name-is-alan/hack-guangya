<script setup lang="ts">
import { reactive } from 'vue'
import { LinkOutlined, PlusOutlined } from '@antdv-next/icons'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const form = reactive({ open: false, loading: false, url: '', label: '' })

function open() {
  Object.assign(form, { open: true, url: '', label: '' })
}

async function submit() {
  if (!form.url.trim()) {
    message.warning('请输入分享链接')
    return
  }
  form.loading = true
  try {
    await bridge.invoke('save_share_link', {
      url: form.url.trim(),
      label: form.label.trim(),
    })
    form.open = false
    await session.refreshState()
    message.success('分享链接已收藏')
  }
  catch (reason) {
    message.error(errorText(reason))
  }
  finally {
    form.loading = false
  }
}
</script>

<template>
  <a-button type="primary" @click="open"><template #icon><PlusOutlined /></template>收藏链接</a-button>
  <a-modal v-model:open="form.open" title="收藏分享链接" :confirm-loading="form.loading" ok-text="收藏" cancel-text="取消" @ok="submit">
    <a-form layout="vertical">
      <a-form-item label="分享链接" required>
        <a-input v-model:value="form.url" placeholder="https://www.guangyapan.com/s/…">
          <template #prefix><LinkOutlined /></template>
        </a-input>
      </a-form-item>
      <a-form-item label="名称"><a-input v-model:value="form.label" placeholder="便于识别的名称（可选）" /></a-form-item>
    </a-form>
  </a-modal>
</template>

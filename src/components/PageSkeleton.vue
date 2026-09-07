<script setup lang="ts">
import { useI18n } from "../i18n";

defineProps<{ title?: string }>();
const { t } = useI18n();
</script>

<!-- 设置/数据未加载完成时的占位：标题 + 居中 spinner。
     此前 v-else 分支渲染纯白页，慢机上像应用卡死（2026-09-07 评审必改 1）。 -->
<template>
  <div class="page">
    <h1>{{ title ?? t("common.loading") }}</h1>
    <p class="page-sub">{{ t("common.loading") }}</p>
    <div class="skeleton-wrap">
      <span class="spinner" role="status" :aria-label="t('common.loading')"></span>
    </div>
  </div>
</template>

<style scoped>
.skeleton-wrap {
  display: flex;
  justify-content: center;
  padding: 64px 0;
}

.spinner {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 2.5px solid var(--border);
  border-top-color: var(--accent);
  animation: skeleton-spin 0.8s linear infinite;
}

@keyframes skeleton-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>

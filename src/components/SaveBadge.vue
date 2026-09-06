<script setup lang="ts">
import { useI18n } from "../i18n";

defineProps<{
  state: "idle" | "saving" | "saved" | "error";
  error?: string;
}>();

const { t } = useI18n();
</script>

<template>
  <span
    v-if="state !== 'idle'"
    class="save-badge"
    :class="state"
    :title="state === 'error' ? error : undefined"
  >
    {{
      state === "saving"
        ? t("common.saving")
        : state === "saved"
          ? t("common.saved")
          : t("common.save_failed")
    }}
  </span>
</template>

<style scoped>
.save-badge {
  display: inline-flex;
  align-items: center;
  padding: 3px 12px;
  font-size: 12px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: var(--panel-2);
  color: var(--text-secondary);
  white-space: nowrap;
  transition: opacity 0.2s ease;
}

.save-badge.saved {
  color: var(--ok, #3a9d5d);
  border-color: var(--ok, #3a9d5d);
}

.save-badge.error {
  color: var(--fail, #d25b5a);
  border-color: var(--fail, #d25b5a);
}
</style>

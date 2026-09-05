<script setup lang="ts">
import { computed, ref } from "vue";
import { api } from "../api";
import { useI18n } from "../i18n";
import type { AppSettings, ButtonAction, RemoteButtonId } from "../types";
import RemoteCanvas from "../components/RemoteCanvas.vue";
import ActionPicker from "../components/ActionPicker.vue";

const props = defineProps<{
  settings: AppSettings | null;
  saveTick: number;
}>();

const emit = defineEmits<{ "update-settings": [settings: AppSettings] }>();
const { t } = useI18n();

const selectedButton = ref<RemoteButtonId | null>(null);
const editingSlot = ref<"single" | "double" | "long">("single");
const showPicker = ref(false);
const newProfileName = ref("");
const foregroundProcess = ref<string | null>(null);

const SECONDARY_BUTTONS = new Set(["home", "menu", "ok", "tv"]);

const buttonNames: Record<RemoteButtonId, string> = {
  power: "电源", up: "上", left: "左", ok: "OK", right: "右", down: "下",
  back: "返回", volume_up: "音量+", home: "主页", volume_down: "音量−",
  menu: "菜单", tv: "TV",
};

const activeProfileId = computed(() => props.settings?.profiles.selectedProfileId ?? "");

const activeProfile = computed(
  () => props.settings?.profiles.profiles.find((p) => p.id === activeProfileId.value) ?? null,
);

function bindingFor(button: RemoteButtonId) {
  return activeProfile.value?.mapping.bindings[button] ?? { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
}

function actionLabel(action: ButtonAction): string {
  switch (action.kind) {
    case "disabled": return t("buttons.action.disabled");
    case "shortcut": return action.label;
    case "media_key": return { play_pause: "播放/暂停", stop: "停止", next: "下一曲", previous: "上一曲", mute: "静音" }[action.code];
    case "volume_up": return "音量+";
    case "volume_down": return "音量−";
    case "volume_mute": return "静音";
    case "open_app": return `打开 ${action.label}`;
    case "open_url": return `打开 ${action.url}`;
    case "screenshot": return action.region ? "区域截图" : "全屏截图";
    case "show_desktop": return "显示桌面";
    case "task_view": return "任务视图";
    case "app_switcher": return "切换应用";
    case "click_confirm": return "点击确认";
    case "open_settings": return "打开声桥";
    case "custom": return action.shortcut.label;
  }
}

function mutateProfiles(mutator: (profiles: AppSettings["profiles"]) => void) {
  if (!props.settings) return;
  const profiles = JSON.parse(JSON.stringify(props.settings.profiles)) as AppSettings["profiles"];
  mutator(profiles);
  emit("update-settings", { ...props.settings, profiles });
}

function selectButton(button: string) {
  selectedButton.value = button as RemoteButtonId;
  editingSlot.value = "single";
}

function changeAction(slot: "single" | "double" | "long") {
  if (!selectedButton.value) return;
  editingSlot.value = slot;
  showPicker.value = true;
}

function applyAction(action: ButtonAction) {
  const button = selectedButton.value;
  if (!button || !activeProfileId.value) return;
  mutateProfiles((profiles) => {
    const profile = profiles.profiles.find((p) => p.id === activeProfileId.value);
    if (!profile) return;
    const bindings = profile.mapping.bindings;
    if (!bindings[button]) bindings[button] = { single: { kind: "disabled" }, double: { kind: "disabled" }, long: { kind: "disabled" } };
    bindings[button][editingSlot.value] = action;
  });
  showPicker.value = false;
}

function selectProfile(id: string) {
  mutateProfiles((profiles) => {
    profiles.selectedProfileId = id;
  });
}

function addProfile() {
  const name = newProfileName.value.trim();
  if (!name) return;
  mutateProfiles((profiles) => {
    const id = `profile-${Date.now().toString(36)}`;
    profiles.profiles.push({
      id, name, icon: "",
      mapping: { bindings: {} },
    });
    profiles.selectedProfileId = id;
  });
  newProfileName.value = "";
}

function removeProfile(id: string) {
  if (!props.settings || props.settings.profiles.profiles.length <= 1) return;
  mutateProfiles((profiles) => {
    profiles.profiles = profiles.profiles.filter((p) => p.id !== id);
    profiles.rules.processBindings = Object.fromEntries(
      Object.entries(profiles.rules.processBindings).filter(([, v]) => v !== id),
    );
    if (!profiles.profiles.find((p) => p.id === profiles.selectedProfileId)) {
      profiles.selectedProfileId = profiles.profiles[0]?.id ?? "";
    }
    if (profiles.rules.fallbackProfileId === id) profiles.rules.fallbackProfileId = "";
  });
  if (selectedButton.value) selectButton(selectedButton.value);
}

function toggleSmart(enabled: boolean) {
  mutateProfiles((profiles) => {
    profiles.smartEnabled = enabled;
  });
}

function toggleMapping(enabled: boolean) {
  if (!props.settings) return;
  emit("update-settings", { ...props.settings, buttonMappingEnabled: enabled });
}

async function bindCurrentApp() {
  if (!activeProfileId.value) return;
  const process = await api.getForegroundProcess();
  if (!process) return;
  foregroundProcess.value = process;
  await api.bindProcessToProfile(process, activeProfileId.value);
  // 绑定在 Rust 侧直接改了设置，重新拉取同步到前端。
  const settings = await api.getSettings();
  emit("update-settings", settings);
}

async function unbindApp(process: string) {
  await api.unbindProcess(process);
  const settings = await api.getSettings();
  emit("update-settings", settings);
}

async function probeForeground() {
  foregroundProcess.value = await api.getForegroundProcess();
}

const smartBindings = computed(() =>
  Object.entries(props.settings?.profiles.rules.processBindings ?? {}),
);
</script>

<template>
  <div class="page" v-if="settings">
    <h1>{{ t("buttons.title") }}</h1>
    <p class="page-sub">{{ t("buttons.canvas.hint") }}</p>

    <section class="card">
      <div class="setting-row" style="padding-top: 0">
        <div>
          <div class="label">{{ t("buttons.mapping.toggle") }}</div>
          <div class="desc">{{ t("buttons.mapping.hint") }}</div>
        </div>
        <button
          class="switch"
          :class="{ on: settings.buttonMappingEnabled }"
          @click="toggleMapping(!settings.buttonMappingEnabled)"
        ></button>
      </div>
    </section>

    <section class="card">
      <h3>{{ t("buttons.profile") }}</h3>
      <div class="row" style="margin-bottom: 10px">
        <button
          v-for="profile in settings.profiles.profiles"
          :key="profile.id"
          class="btn"
          :class="{ primary: profile.id === activeProfileId }"
          @click="selectProfile(profile.id)"
        >
          {{ profile.name }}
        </button>
      </div>
      <div class="row">
        <input v-model="newProfileName" type="text" :placeholder="t('buttons.profile.new')" style="width: 160px" @keydown.enter="addProfile" />
        <button class="btn" @click="addProfile">{{ t("common.add") }}</button>
        <span class="spacer"></span>
        <button
          v-if="settings.profiles.profiles.length > 1"
          class="btn danger"
          @click="removeProfile(activeProfileId)"
        >
          {{ t("common.delete") }}
        </button>
      </div>
    </section>

    <section class="card">
      <div class="setting-row" style="padding-top: 0">
        <div>
          <div class="label">{{ t("buttons.smart") }}</div>
          <div class="desc">{{ t("buttons.smart.hint") }}</div>
        </div>
        <button
          class="switch"
          :class="{ on: settings.profiles.smartEnabled }"
          @click="toggleSmart(!settings.profiles.smartEnabled)"
        ></button>
      </div>
      <template v-if="settings.profiles.smartEnabled">
        <div class="row" style="margin: 8px 0">
          <button class="btn" @click="probeForeground">
            {{ t("buttons.smart.bind_current") }}
          </button>
          <span v-if="foregroundProcess" class="badge">{{ foregroundProcess }} → {{ activeProfile?.name }}</span>
        </div>
        <div v-if="smartBindings.length" class="history-list">
          <div v-for="[process, profileId] in smartBindings" :key="process" class="history-item">
            <div>
              <strong>{{ process }}</strong>
              <span style="color: var(--text-secondary)">
                → {{ settings.profiles.profiles.find((p) => p.id === profileId)?.name ?? "?" }}
              </span>
            </div>
            <button class="btn subtle" @click="unbindApp(process)">✕</button>
          </div>
        </div>
      </template>
    </section>

    <div v-if="activeProfile" style="display: grid; grid-template-columns: 260px 1fr; gap: 14px; align-items: start">
      <section class="card" style="margin-bottom: 0">
        <RemoteCanvas
          :selected="selectedButton"
          :recording="false"
          @select="selectButton"
        />
      </section>

      <section class="card" style="margin-bottom: 0">
        <template v-if="selectedButton">
          <h3>{{ buttonNames[selectedButton] }}</h3>
          <div
            v-for="slot in (SECONDARY_BUTTONS.has(selectedButton) ? ['single', 'double', 'long'] : ['single'])"
            :key="slot"
            class="setting-row"
          >
            <div>
              <div class="label">{{ t(`buttons.slot.${slot}` as never) }}</div>
              <div class="desc">{{ actionLabel(bindingFor(selectedButton)[slot as 'single' | 'double' | 'long']) }}</div>
            </div>
            <button class="btn" @click="changeAction(slot as 'single' | 'double' | 'long')">
              {{ t("buttons.action.change") }}
            </button>
          </div>
          <p v-if="!SECONDARY_BUTTONS.has(selectedButton)" class="hint" style="margin-top: 10px">
            {{ t("buttons.slot.locked") }}
          </p>
        </template>
        <div v-else class="empty">{{ t("buttons.canvas.hint") }}</div>
      </section>
    </div>

    <ActionPicker
      v-if="showPicker && selectedButton"
      :button-id="selectedButton"
      :slot="editingSlot"
      :current="bindingFor(selectedButton)[editingSlot]"
      @pick="applyAction"
      @close="showPicker = false"
    />
  </div>
  <div v-else class="page">
    <p class="empty">{{ t("common.loading") }}</p>
  </div>
</template>

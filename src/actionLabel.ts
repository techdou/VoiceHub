import type { useI18n } from "./i18n";
import type { ButtonAction } from "./types";

type T = ReturnType<typeof useI18n>["t"];

/// 动作显示名（i18n 统一出口，ButtonsPage / ActionPicker 共用）。
export function actionLabel(action: ButtonAction, t: T): string {
  switch (action.kind) {
    case "disabled":
      return t("buttons.action.disabled");
    case "shortcut":
      return action.label;
    case "media_key":
      return action.code === "play_pause"
        ? t("action.media.play_pause")
        : action.code === "next"
          ? t("action.media.next")
          : action.code === "previous"
            ? t("action.media.previous")
            : t("action.volume_mute");
    case "volume_up":
      return t("action.volume_up");
    case "volume_down":
      return t("action.volume_down");
    case "volume_mute":
      return t("action.volume_mute");
    case "open_app":
      return `${t("buttons.action.tab.apps")}: ${action.label}`;
    case "open_url":
      return `${t("buttons.action.tab.apps")}: ${action.url}`;
    case "screenshot":
      return action.region ? t("action.screenshot_region") : t("action.screenshot_full");
    case "show_desktop":
      return t("action.show_desktop");
    case "task_view":
      return t("action.task_view");
    case "app_switcher":
      return t("action.app_switcher");
    case "click_confirm":
      return t("action.click_confirm");
    case "open_settings":
      return t("action.open_settings");
    case "custom":
      return action.shortcut.label;
  }
}

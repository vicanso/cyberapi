import { defineStore } from "pinia";
import { appWindow } from "@tauri-apps/api/window";
import localforage from "localforage";

import { isWebMode } from "../helpers/util";

const settingStore = localforage.createInstance({
  name: "setting",
});

interface SettingStore {
  theme: string;
}

export const useSettingStore = defineStore("common", {
  state: () => {
    return {
      fetching: false,
      theme: "",
      isDark: false,
    };
  },
  actions: {
    async fetch(): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        // 优先使用用户设置
        const setting = await settingStore.getItem<SettingStore>("setting");
        let theme = setting?.theme || "";
        if (!isWebMode() && !theme) {
          const result = await appWindow.theme();
          theme = (result as string) || "";
        }
        this.isDark = theme == "dark";
        this.theme = theme;
      } catch (err) {
        // 获取失败则忽略
      } finally {
        this.fetching = false;
      }
    },
  },
});

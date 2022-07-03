import { defineStore } from "pinia";
import { appWindow } from "@tauri-apps/api/window";
import localforage from "localforage";

import { isWebMode } from "../helpers/util";

const settingStore = localforage.createInstance({
  name: "setting",
});

export const useCommonStore = defineStore("common", {
  state: () => {
    return {
      setting: {
        theme: "",
      },
    };
  },
  actions: {
    async getSetting(): Promise<void> {
      let theme = "";
      if (!isWebMode()) {
        try {
          // 优先使用用户设置
          const setting = await settingStore.getItem<{
            theme: string;
          }>("setting");
          theme = setting?.theme || "";
          if (!theme) {
            const result = await appWindow.theme();
            theme = (result as string) || "";
          }
        } catch (err) {
          // 获取失败则忽略
        }
      }
      this.setting.theme = theme;
    },
  },
});

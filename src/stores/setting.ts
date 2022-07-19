import { defineStore } from "pinia";
import { appWindow } from "@tauri-apps/api/window";
import localforage from "localforage";

import { isWebMode, getBodyWidth } from "../helpers/util";

const settingStore = localforage.createInstance({
  name: "setting",
});

interface AppSetting {
  theme: string;
  collectionSortType: string;
  collectionColumnWidths: number[];
  size: {
    width: number;
    height: number;
  };
}

const appSettingKey = "app";

async function getAppSetting(): Promise<AppSetting> {
  const setting = await settingStore.getItem<AppSetting>(appSettingKey);
  if (setting) {
    return setting;
  }
  return {} as AppSetting;
}

function updateAppSetting(data: AppSetting): Promise<AppSetting> {
  return settingStore.setItem(appSettingKey, data);
}

function isDarkTheme(theme: string) {
  return theme === "dark";
}

export const useSettingStore = defineStore("common", {
  state: () => {
    return {
      fetching: false,
      theme: "",
      isDark: false,
      systemTheme: "",
      collectionSortType: "",
      // collection页面的分栏宽度
      collectionColumnWidths: [] as number[],
      // 展示尺寸
      size: {
        width: 0,
        height: 0,
      },
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
        const setting = await getAppSetting();
        let theme = setting.theme || "";
        if (setting.collectionColumnWidths?.length) {
          this.collectionColumnWidths = setting.collectionColumnWidths;
        }
        if (!isWebMode()) {
          const result = await appWindow.theme();
          this.systemTheme = (result as string) || "";
          if (!theme) {
            theme = this.systemTheme;
          }
        }
        this.isDark = isDarkTheme(theme);
        this.theme = theme;
        this.collectionSortType = setting.collectionSortType;

        let sum = 0;
        this.collectionColumnWidths.forEach((item) => {
          sum += item;
        });
        const bodyWidth = getBodyWidth();

        // 如果为空或者最后侧宽度太少
        // 设置默认值
        if (!this.collectionColumnWidths.length || sum > bodyWidth - 200) {
          // 左侧，中间，右侧自动填充
          const first = 300;
          this.collectionColumnWidths = [first, (bodyWidth - first) >> 1];
        }
        if (setting.size) {
          this.size = setting.size;
        }
      } catch (err) {
        // 获取失败则忽略
      } finally {
        this.fetching = false;
      }
    },
    async updateTheme(theme: string) {
      const setting = await getAppSetting();
      setting.theme = theme;
      await updateAppSetting(setting);
      this.theme = theme;
      // 如果theme 为空表示使用系统主题
      this.isDark = isDarkTheme(theme || this.systemTheme);
    },
    async updateCollectionSortType(sortType: string) {
      const setting = await getAppSetting();
      setting.collectionSortType = sortType;
      await updateAppSetting(setting);
      this.collectionSortType = sortType;
    },
    async updateCollectionColumnWidths(widths: number[]) {
      const setting = await getAppSetting();
      setting.collectionColumnWidths = widths;
      await updateAppSetting(setting);
      this.collectionColumnWidths = widths;
    },
    async updateSize(width: number, height: number) {
      const setting = await getAppSetting();
      setting.size = {
        width,
        height,
      };
      await updateAppSetting(setting);
      this.size = {
        width,
        height,
      };
    },
  },
});

import { defineStore } from "pinia";
import { appWindow } from "@tauri-apps/api/window";

import { setWindowSize } from "../commands/window";

import { isWebMode } from "../helpers/util";
import { LocationQuery, RouteParams } from "vue-router";
import { getSettingStore } from "./local";

export interface Timeout {
  connect: number;
  write: number;
  read: number;
}

interface AppSetting {
  theme: string;
  collectionSortType: string;
  collectionColumnWidths: number[];
  resizeType: string;
  size: {
    width: number;
    height: number;
  };
  latestRoute: {
    name: string;
    params: RouteParams;
    query: LocationQuery;
  };
  timeout: Timeout;
}

export enum ResizeType {
  Max = "max",
  Custom = "custom",
}

const appSettingKey = "app";

async function getAppSetting(): Promise<AppSetting> {
  const setting = await getSettingStore().getItem<AppSetting>(appSettingKey);
  if (setting) {
    return setting;
  }
  return {} as AppSetting;
}

function updateAppSetting(data: AppSetting): Promise<AppSetting> {
  return getSettingStore().setItem(appSettingKey, data);
}

export async function updateAppLatestRoute(route: {
  name: string;
  params: RouteParams;
  query: LocationQuery;
}) {
  const setting = await getAppSetting();
  setting.latestRoute = route;
  await updateAppSetting(setting);
}

export async function getAppLatestRoute() {
  const setting = await getAppSetting();
  return setting.latestRoute;
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
      resizeType: "",
      // 展示尺寸
      size: {
        width: 0,
        height: 0,
      },
      timeout: {
        connect: 0,
        read: 0,
        write: 0,
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
        let currentTheme = setting.theme || "";
        this.theme = currentTheme;
        if (setting.collectionColumnWidths?.length) {
          this.collectionColumnWidths = setting.collectionColumnWidths;
        }
        if (!isWebMode()) {
          const result = await appWindow.theme();
          this.systemTheme = (result as string) || "";
          if (!currentTheme) {
            currentTheme = this.systemTheme;
          }
        }
        this.isDark = isDarkTheme(currentTheme);
        this.collectionSortType = setting.collectionSortType;

        // 如果为空
        // 设置默认值
        if (!this.collectionColumnWidths.length) {
          // 左侧，中间，右侧自动填充
          const first = 300;
          this.collectionColumnWidths = [first, 0.5];
        }
        if (setting.size) {
          this.size = setting.size;
        }
        this.resizeType = setting.resizeType || ResizeType.Max;
        this.timeout = Object.assign(
          {
            connect: 0,
            write: 0,
            read: 0,
          },
          setting.timeout
        );
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
    async updateParamsColumnWidth(width: number) {
      if (width < 0.2 || width > 0.8) {
        return;
      }
      const setting = await getAppSetting();
      const widths = setting.collectionColumnWidths.slice(0);
      widths[1] = width;
      return this.updateCollectionColumnWidths(widths);
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
    async updateResizeType(resizeType: string) {
      const setting = await getAppSetting();
      setting.resizeType = resizeType;
      await updateAppSetting(setting);
      this.resizeType = resizeType;
    },
    async resize() {
      const { width, height } = this.size;
      if (this.resizeType === ResizeType.Max) {
        await setWindowSize(-1, -1);
      } else if (width > 0 && height > 0) {
        await setWindowSize(width, height);
      }
    },
    getRequestTimeout() {
      return this.timeout;
    },
    async updateRequestTimeout(params: Timeout) {
      const setting = await getAppSetting();
      setting.timeout = params;
      await updateAppSetting(setting);
      this.timeout = setting.timeout;
    },
  },
});

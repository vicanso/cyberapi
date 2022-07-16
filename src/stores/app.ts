import { defineStore } from "pinia";
import { getVersion, getTauriVersion } from "@tauri-apps/api/app";
import { arch, platform, type, version } from "@tauri-apps/api/os";

import { isWebMode } from "../helpers/util";

export const useAppStore = defineStore("app", {
  state: () => {
    return {
      version: "--",
      tauriVersion: "--",
      arch: "--",
      platform: "--",
      os: "--",
      osVersion: "--",
    };
  },
  actions: {
    async fetch() {
      if (!isWebMode()) {
        this.version = await getVersion();
        this.tauriVersion = await getTauriVersion();
        this.arch = await arch();
        this.platform = await platform();
        this.os = await type();
        this.osVersion = await version();
      }
    },
  },
});

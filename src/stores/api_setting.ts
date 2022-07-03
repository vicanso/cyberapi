import { defineStore } from "pinia";

import {
  APISetting,
  listAPISetting,
  newDefaultAPISetting,
  createAPISetting,
} from "../commands/api_setting";

export const useAPISettingsStore = defineStore("apiSettings", {
  state: () => {
    return {
      count: -1,
      apiSettings: [] as APISetting[],
      processing: false,
    };
  },
  actions: {
    async add(folder: string = "") {
      if (this.processing) {
        return;
      }
      const setting = newDefaultAPISetting();
      this.processing = true;
      setting.folder = folder;
      try {
        await createAPISetting(setting);
      } finally {
        this.processing = false;
      }
    },
    async list(): Promise<void> {
      if (this.processing) {
        return;
      }
      this.processing = true;
      try {
        const resp = await listAPISetting();
        this.apiSettings = resp;
        this.count = resp.length;
      } finally {
        this.processing = false;
      }
    },
  },
});

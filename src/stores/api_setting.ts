import { defineStore } from "pinia";

import { APISetting, listAPISetting } from "../commands/api_setting";

export const useAPISettingsStore = defineStore("apiSettings", {
  state: () => {
    return {
      count: -1,
      apiSettings: [] as APISetting[],
      processing: false,
    };
  },
  actions: {
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

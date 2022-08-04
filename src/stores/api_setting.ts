import { defineStore } from "pinia";
import dayjs from "dayjs";

import {
  APISetting,
  listAPISetting,
  createAPISetting,
  updateAPISetting,
  deleteAPISettings,
} from "../commands/api_setting";

export enum SettingType {
  HTTP = "http",
  Folder = "folder",
}

export const useAPISettingStore = defineStore("apiSettings", {
  state: () => {
    return {
      apiSettings: [] as APISetting[],
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    findByID(id: string): APISetting {
      const index = this.apiSettings.findIndex((item) => item.id === id);
      return this.apiSettings[index];
    },
    async updateByID(id: string, data: unknown) {
      const index = this.apiSettings.findIndex((item) => item.id === id);
      const item = Object.assign(this.apiSettings[index], data);
      await this.update(item);
    },
    async add(data: APISetting) {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPISetting(data);
        const arr = this.apiSettings.slice(0);
        arr.push(data);
        this.apiSettings = arr;
      } finally {
        this.adding = false;
      }
    },
    async fetch(): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        this.apiSettings = await listAPISetting();
      } finally {
        this.fetching = false;
      }
    },
    async update(data: APISetting) {
      if (this.updating) {
        return;
      }
      this.updating = true;
      try {
        data.updatedAt = dayjs().format();
        await updateAPISetting(data);
        const arr = this.apiSettings.slice(0);
        let found = -1;
        arr.forEach((item, index) => {
          if (item.id === data.id) {
            found = index;
          }
        });
        if (found !== -1) {
          arr[found] = data;
        }
        this.apiSettings = arr;
      } finally {
        this.updating = false;
      }
    },
    async remove(id: string) {
      if (this.removing) {
        return;
      }
      this.removing = true;
      try {
        await deleteAPISettings([id]);
        this.apiSettings = this.apiSettings.filter((item) => item.id !== id);
      } finally {
        this.removing = false;
      }
    },
  },
});

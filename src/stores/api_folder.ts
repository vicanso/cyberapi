import dayjs from "dayjs";
import { ulid } from "ulid";
import { defineStore } from "pinia";
import {
  createAPIFolder,
  listAPIFolder,
  updateAPIFolder,
} from "../commands/api_folder";
import { APIFolder } from "../commands/api_folder";

export const useAPIFoldersStore = defineStore("apiFolders", {
  state: () => {
    return {
      apiFolders: [] as APIFolder[],
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    async add(data: APIFolder): Promise<void> {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPIFolder(data);
        const arr = this.apiFolders.slice(0);
        arr.push(data);
        this.apiFolders = arr;
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
        this.apiFolders = await listAPIFolder();
      } finally {
        this.fetching = false;
      }
    },
    async update(data: APIFolder): Promise<void> {
      if (this.updating) {
        return;
      }
      this.updating = true;
      data.updatedAt = dayjs().format();
      try {
        await updateAPIFolder(data);
        const arr = this.apiFolders.slice(0);
        const index = arr.findIndex((item) => item.id === data.id);
        if (index !== -1) {
          arr[index] = data;
        }
        this.apiFolders = arr;
      } finally {
        this.updating = false;
      }
    },
    async remove(id: string): Promise<void> {
      if (this.removing) {
        return;
      }
    },
  },
});

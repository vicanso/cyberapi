import { defineStore } from "pinia";

import {
  APICollection,
  createAPICollection,
  updateAPICollection,
  listAPICollection,
  deleteAPICollection,
} from "../commands/api_collection";

export const useAPICollectionsStore = defineStore("apiCollections", {
  state: () => {
    return {
      apiCollections: [] as APICollection[],
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    async add(data: APICollection): Promise<void> {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPICollection(data);
        const arr = this.apiCollections.slice(0);
        arr.push(data);
        this.apiCollections = arr;
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
        this.apiCollections = await listAPICollection();
      } finally {
        this.fetching = false;
      }
    },
    async update(data: APICollection): Promise<void> {
      if (this.updating) {
        return;
      }
      this.updating = true;
      try {
        await updateAPICollection(data);
        const arr = this.apiCollections.slice(0);
        const index = arr.findIndex((item) => item.id === data.id);
        if (index !== -1) {
          arr[index] = data;
        }
        this.apiCollections = arr;
      } finally {
        this.updating = false;
      }
    },
    async remove(id: string): Promise<void> {
      if (this.removing) {
        return;
      }
      this.removing = true;
      try {
        await deleteAPICollection(id);
        const arr = this.apiCollections.slice(0);
        const index = arr.findIndex((item) => item.id === id);
        if (index !== -1) {
          arr.splice(index, 1);
        }
        this.apiCollections = arr;
      } finally {
        this.removing = false;
      }
    },
  },
});

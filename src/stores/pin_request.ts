import { defineStore } from "pinia";
import { getPinRequestStore } from "./local";

interface LatestRequest {
  id: string;
}

export const usePinRequestStore = defineStore("pinRequest", {
  state: () => {
    return {
      fetching: false,
      currentCollection: "",
      requests: [] as LatestRequest[],
    };
  },
  actions: {
    async fetch(collection: string) {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        const result = await getPinRequestStore().getItem(collection);
        this.requests = (result || []) as LatestRequest[];
        this.currentCollection = collection;
      } finally {
        this.fetching = false;
      }
    },
    async save() {
      const { currentCollection, requests } = this;
      const arr = requests.map((item) => Object.assign({}, item));
      await getPinRequestStore().setItem(currentCollection, arr);
    },
    async add(collection: string, req: LatestRequest) {
      const { currentCollection, requests } = this;
      if (currentCollection !== collection) {
        return;
      }
      const found = requests.find((item) => item.id === req.id);
      if (found) {
        return;
      }
      requests.push(req);
      await this.save();
    },
    async remove(id: string) {
      const { requests } = this;
      const arr = requests.filter((item) => item.id !== id);
      this.requests = arr;
      await this.save();
    },
  },
});

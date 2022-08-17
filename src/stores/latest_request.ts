import { defineStore } from "pinia";
import { getLatestRequestStore } from "./local";

interface LatestRequest {
  name: string;
  id: string;
}

const max = 10;

export const useLatestRequestStore = defineStore("latestRequest", {
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
        const result = await getLatestRequestStore().getItem(collection);
        if (result) {
          this.requests = result as LatestRequest[];
        }
        this.currentCollection = collection;
      } finally {
        this.fetching = false;
      }
    },
    async save() {
      const { currentCollection, requests } = this;
      const arr = requests.map((item) => Object.assign({}, item));
      await getLatestRequestStore().setItem(currentCollection, arr);
    },
    async add(collection: string, req: LatestRequest) {
      const { currentCollection, requests } = this;
      if (currentCollection !== collection) {
        return;
      }
      // TODO 根据次数与时间清除
      // 已存在，不处理
      const found = requests.find((item) => item.id === req.id);
      if (found) {
        return;
      }
      requests.push(req);
      if (requests.length > max) {
        requests.shift();
      }
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

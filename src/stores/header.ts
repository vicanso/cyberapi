import { defineStore } from "pinia";

interface Breadcrumb {
  route: string;
  name: string;
}

export const useHeaderStore = defineStore("header", {
  state: () => {
    return {
      breadcrumbs: [] as Breadcrumb[],
    };
  },
  actions: {
    add(breadcrumb: Breadcrumb) {
      const arr = this.breadcrumbs.slice(0);
      arr.push(breadcrumb);
      this.breadcrumbs = arr;
    },
    clear() {
      this.breadcrumbs = [];
    },
  },
});

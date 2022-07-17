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
      const { breadcrumbs } = this;
      if (
        breadcrumbs.length &&
        breadcrumbs[breadcrumbs.length - 1].route === breadcrumb.route
      ) {
        return;
      }
      const arr = breadcrumbs.slice(0);
      arr.push(breadcrumb);
      this.breadcrumbs = arr;
    },
    clear() {
      this.breadcrumbs = [];
    },
  },
});

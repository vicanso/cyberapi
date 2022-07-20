import { defineStore } from "pinia";

import {
  listCookie,
  Cookie,
  deleteCookie,
  addCookie,
} from "../commands/cookies";

export const useCookieStore = defineStore("cookie", {
  state: () => {
    return {
      cookies: [] as Cookie[],
      fetching: false,
      removing: false,
      adding: false,
    };
  },
  actions: {
    async fetch() {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        this.cookies = await listCookie();
      } finally {
        this.fetching = false;
      }
    },
    async remove(c: Cookie) {
      if (this.removing) {
        return;
      }
      this.removing = true;
      try {
        await deleteCookie(c);
        const cookies = this.cookies.filter((item) => {
          return (
            item.name === c.name &&
            item.domain === c.domain &&
            item.path === c.path
          );
        });
        this.cookies = cookies;
      } finally {
        this.removing = false;
      }
    },
    async add(c: Cookie) {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await addCookie(c);
        const arr = this.cookies.slice(0);
        arr.push(c);
        this.cookies = arr;
      } finally {
        this.adding = false;
      }
    },
  },
});

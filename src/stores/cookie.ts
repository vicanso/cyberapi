import { defineStore } from "pinia";

import {
  listCookie,
  Cookie,
  deleteCookie,
  addOrUpdate,
} from "../commands/cookies";

function isSameCookie(c1: Cookie, c2: Cookie) {
  return c1.name === c2.name && c1.domain === c2.domain && c1.path === c2.path;
}

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
        const cookies = this.cookies.slice(0).filter((item) => {
          return !isSameCookie(item, c);
        });
        this.cookies = cookies;
      } finally {
        this.removing = false;
      }
    },
    async addOrUpdate(c: Cookie) {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await addOrUpdate(c);
        const arr = this.cookies.slice(0);
        let found = -1;
        arr.forEach((item, index) => {
          if (isSameCookie(item, c)) {
            found = index;
          }
        });
        if (found === -1) {
          arr.push(c);
        } else {
          arr[found] = c;
        }
        this.cookies = arr;
      } finally {
        this.adding = false;
      }
    },
  },
});

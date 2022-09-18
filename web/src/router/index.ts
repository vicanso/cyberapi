import { createRouter, createWebHistory } from "vue-router";
import { names, routes } from "./routes";

const history = createWebHistory();

const router = createRouter({
  history,
  routes,
});

export function goToDoc(name: string) {
  router.push({
    name: names.docs,
    params: {
      name,
    },
  });
}

export function goToHome() {
  router.push({
    name: names.home,
  });
}

export default router;

import {
  createRouter,
  createWebHashHistory,
  LocationQueryRaw,
} from "vue-router";
import { routes } from "./routes";

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

export function goTo(
  name: string,
  params?: {
    replace?: boolean;
    query?: LocationQueryRaw;
  }
): void {
  router.push({
    name,
    replace: params?.replace,
    query: params?.query,
  });
}

type loadingEvent = () => void;

const noop = function () {
  return;
};

let startEvent = noop;
let finishEvent = noop;

export function setLoadingEvent(
  start: loadingEvent,
  finish: loadingEvent
): void {
  startEvent = start;
  finishEvent = finish;
}

router.beforeEach(function (to, from, next) {
  if (!from || to.path !== from.path) {
    startEvent();
  }
  next();
});

router.afterEach(function (to, from) {
  if (!from || to.path !== from.path) {
    finishEvent();
  }
});
export default router;

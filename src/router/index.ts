import { createRouter, createWebHashHistory } from "vue-router";
import { routes } from "./routes";

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

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

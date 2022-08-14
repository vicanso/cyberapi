import { createApp } from "vue";
import { create } from "naive-ui";
import { createPinia } from "pinia";
import Debug from "debug";
import Root from "./Root";
import router, { goTo } from "./router";
import { isWebMode } from "./helpers/util";
import { changeI18nLocale } from "./i18n";
import { getAppLatestRoute } from "./stores/setting";

// web mode enable debug:*
if (isWebMode()) {
  Debug.enable("*");
}

const app = createApp(Root);

async function init() {
  changeI18nLocale("zh");
  app.use(router);
  // 非浏览器模式打开上次打开的页面
  if (!isWebMode()) {
    const route = await getAppLatestRoute();
    if (route.name) {
      goTo(route.name, {
        query: route.query,
      });
    }
  }
}

const naive = create();
init()
  // TODO
  // 初始化失败是否弹窗
  .catch(console.error)
  .finally(() => {
    app.use(naive).use(createPinia()).mount("#app");
  });

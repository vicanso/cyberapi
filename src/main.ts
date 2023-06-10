import { createApp } from "vue";
import { create } from "naive-ui";
import { createPinia } from "pinia";
import { message } from "@tauri-apps/api/dialog";

import Debug from "debug";
import router, { goTo } from "./router";
import Root from "./root";
import { isWebMode } from "./helpers/util";
import { changeI18nLocale, LANG } from "./i18n";
import { getAppLatestRoute } from "./stores/setting";
import { getLang } from "./stores/local";
import { handleDatabaseCompatible } from "./commands/database";
import { showSplashscreen } from "./commands/window";
import { initWindowEvent } from "./event";
import "./userWorker";

// web mode enable debug:*
if (isWebMode()) {
  Debug.enable("*");
}

const app = createApp(Root);

async function init() {
  initWindowEvent();
  showSplashscreen();
  // TODO 校验数据库版本
  // 判断是否需要升级级别
  await handleDatabaseCompatible();
  const lang = (await getLang()) || LANG.zh;
  changeI18nLocale(lang);
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
  // 初始化失败是否弹窗
  .catch(console.error)
  .finally(() => {
    // TODO 确认客户是否允许提交此类出错信息至服务
    // 便于后续优化
    const unknown = "unknown";
    app.config.errorHandler = (err, instance, info) => {
      const name = instance?.$options.name || unknown;
      const msg = (err as Error).message || unknown;
      const content = `${name}(${msg}): ${info}`;
      if (isWebMode()) {
        console.error(content);
        throw err;
      } else {
        message(content);
      }
    };
    app.use(naive).use(createPinia()).mount("#app");
  });

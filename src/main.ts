import { createApp } from "vue";
import { create } from "naive-ui";
import { createPinia } from "pinia";
import Debug from "debug";
import Root from "./Root";
import router from "./router";
import { isWebMode } from "./helpers/util";
import { changeI18nLocale } from "./i18n";

// web mode enable debug:*
if (isWebMode()) {
  Debug.enable("*");
}

const naive = create();
const app = createApp(Root);
// TODO 初始化
changeI18nLocale("zh");
app.use(router).use(naive).use(createPinia()).mount("#app");

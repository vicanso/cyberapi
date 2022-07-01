import { createApp } from "vue";
import { create } from "naive-ui";
import { createPinia } from "pinia";
import Debug from "debug";
import Root from "./Root";
import { isWebMode } from "./helpers/util";

// web mode enable debug:*
if (isWebMode()) {
  Debug.enable("*");
}

const naive = create();
const app = createApp(Root);
app.use(naive).use(createPinia()).mount("#app");

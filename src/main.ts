import { createApp } from "vue";
import App from "./App.vue";
import Debug from "debug";
import { isWebMode } from "./helpers/util";

// web mode enable debug:*
if (isWebMode()) {
  Debug.enable("*");
}

createApp(App).mount("#app");

import { createApp } from "vue";
import { create } from "naive-ui";
import "./style.css";
import Root from "./root";
import router from "./router";

const naive = create();
createApp(Root).use(router).use(naive).mount("#app");

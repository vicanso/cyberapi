import { createApp } from "vue";
import { create } from "naive-ui";
import "./style.css";
import App from "./App";

const naive = create();
createApp(App).use(naive).mount("#app");

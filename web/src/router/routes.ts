import { Component } from "vue";

import Home from "../views/Home";
import Markdown from "../views/Markdown";

export interface Router {
  path: string;
  name: string;
  component: Component | Promise<Component>;
}

export const names = {
  home: "home",
  docs: "docs",
};

export const routes: Router[] = [
  {
    path: "/",
    name: names.home,
    component: Home,
  },
  {
    path: "/docs/:name",
    name: names.docs,
    component: Markdown,
  },
];

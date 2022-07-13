import { Component } from "vue";

import Dashboard from "../views/Dashboard";

export interface Router {
  path: string;
  name: string;
  component: Component | Promise<Component>;
}

export const names = {
  home: "home",
};

export const routes: Router[] = [
  {
    path: "/",
    name: names.home,
    component: Dashboard,
  },
];

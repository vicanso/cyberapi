import { Component } from "vue";

import Dashboard from "../views/Dashboard";
import Collection from "../views/Collection";

export interface Router {
  path: string;
  name: string;
  component: Component | Promise<Component>;
}

export const names = {
  home: "home",
  collection: "collection",
};

export const routes: Router[] = [
  {
    path: "/",
    name: names.home,
    component: Dashboard,
  },
  {
    path: "/collection",
    name: names.collection,
    component: Collection,
  },
];

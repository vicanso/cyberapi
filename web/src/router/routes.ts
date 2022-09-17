import { Component } from "vue";

export interface Router {
  path: string;
  name: string;
  component: Component | Promise<Component>;
}

export const routes: Router[] = [];

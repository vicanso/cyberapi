import { VariableCategory } from "../commands/variable";
import { newVariableStore } from "./variable";
export const ENVRegexp = /\{\{([\S\s]+)\}\}/;

export const useEnvironmentStore = newVariableStore(
  "environments",
  VariableCategory.Environment,
);

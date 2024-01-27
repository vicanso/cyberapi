import { VariableCategory } from "../commands/variable";
import { newVariableStore } from "./variable";

export const useGlobalReqHeaderStore = newVariableStore(
  "globalReqHeaders",
  VariableCategory.GlobalReqHeaders,
);

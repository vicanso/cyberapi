import { invoke, InvokeArgs } from "@tauri-apps/api/tauri";
import Debug from "debug";
import { isWebMode } from "../helpers/util";

export const cmdAddAPISetting = "add_api_setting";
export const cmdListAPISetting = "list_api_setting";
export const cmdUpdateAPISetting = "update_api_setting";

const debug = Debug("invoke");
export function run<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (isWebMode()) {
    debug("invoke, cmd:%s, args:%o", cmd, args);
    return Promise.resolve(null as T);
  }
  return invoke<T>(cmd, args);
}

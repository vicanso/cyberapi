import { invoke, InvokeArgs } from "@tauri-apps/api/tauri";
import Debug from "debug";
import { isWebMode } from "../helpers/util";

export const cmdAddAPISetting = "add_api_setting";
export const cmdListAPISetting = "list_api_setting";
export const cmdUpdateAPISetting = "update_api_setting";

export const cmdAddAPIFolder = "add_api_folder";
export const cmdListAPIFolder = "list_api_folder";
export const cmdUpdateAPIFolder = "update_api_folder";
export const cmdDoHTTPRequest = "do_http_request";

export const cmdListCookie = "list_cookie";

const debug = Debug("invoke");
export function run<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (isWebMode()) {
    debug("invoke, cmd:%s, args:%o", cmd, args);
    // eslint-disable-next-line
    // @ts-ignore: mock
    return Promise.resolve(null);
  }
  return invoke<T>(cmd, args);
}

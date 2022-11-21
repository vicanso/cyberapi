import { appWindow } from "@tauri-apps/api/window";
import { hide } from "@tauri-apps/api/app";
import { isMacOS } from "./helpers/util";

export async function initWindowEvent() {
  if (!(await isMacOS())) {
    return;
  }
  // appWindow.onCloseRequested((e) => {
  //   e.preventDefault();
  //   // TODO allowlist app.all 设置无效
  //   hide();
  // });
}

import { run } from "./invoke";
import { appWindow, LogicalSize } from "@tauri-apps/api/window";

export function closeSplashscreen() {
  run("close_splashscreen");
}

export async function setWindowSize(width: number, height: number) {
  // 如果有设置小于0，则最大化
  if (width < 0 || height < 0) {
    await appWindow.maximize();
  } else if (width > 0 && height > 0) {
    await appWindow.setSize(new LogicalSize(width, height));
  }
}

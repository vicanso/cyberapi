import { FormRules, MessageApi } from "naive-ui";
import dayjs from "dayjs";
import { get, has } from "lodash-es";
import { appWindow } from "@tauri-apps/api/window";
import { writeText } from "@tauri-apps/api/clipboard";

export function isWebMode() {
  return !window.__TAURI_IPC__;
}

export async function setAppTitle(title: string) {
  if (isWebMode()) {
    return;
  }
  await appWindow.setTitle(title);
}

function formatError(err: Error | unknown): string {
  let message = "";
  if (err instanceof Error) {
    message = err.message;
  } else if (has(err, "message")) {
    message = get(err, "message");
  } else {
    message = err as string;
  }
  return message;
}

export function showError(message: MessageApi, err: Error | unknown): void {
  message.error(formatError(err), {
    duration: 3000,
  });
}

// formatDate 格式化日期
export function formatDate(str: string): string {
  if (!str) {
    return "--";
  }
  return dayjs(str).format("YYYY-MM-DD HH:mm:ss");
}

export function formatSimpleDate(str: string): string {
  if (!str) {
    return "--";
  }
  const now = dayjs();
  const date = dayjs(str);
  if (date.year() === now.year()) {
    return date.format("MM-DD HH:mm");
  }
  return date.format("YYYY-MM-DD");
}

export function getBodyWidth(): number {
  return window?.document?.body?.clientWidth || 800;
}

export function getNormalDialogStyle() {
  const bodyWidth = getBodyWidth();
  const modalWidth = bodyWidth >= 1000 ? bodyWidth * 0.7 : bodyWidth - 200;
  const modalStyle = {
    width: `${modalWidth}px`,
  };
  return modalStyle;
}

export function newRequireRules(keys: string[]) {
  const rules: FormRules = {};
  keys.map((key) => {
    rules[key] = {
      required: true,
      trigger: "blur",
    };
  });
  return rules;
}

export function tryToParseArray(data: string) {
  if (!data) {
    return [];
  }
  const body = data.trim();
  if (body.length <= 2 || body[0] !== "[" || body[body.length - 1] !== "]") {
    return [];
  }
  return JSON.parse(body);
}

export async function writeTextToClipboard(text: string) {
  if (isWebMode()) {
    navigator.clipboard.writeText(text);
    return;
  }
  await writeText(text);
}

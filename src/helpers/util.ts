import { MessageApi } from "naive-ui";
import dayjs from "dayjs";

export function isWebMode() {
  return !window.__TAURI_IPC__;
}

function formatError(err: Error | unknown): string {
  let message = "";
  if (err instanceof Error) {
    message = err.message;
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

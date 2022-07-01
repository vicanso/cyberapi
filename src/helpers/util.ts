import { MessageApi } from "naive-ui";

export function isWebMode() {
  return !window.__TAURI_IPC__;
}

function formatError(err: Error | unknown): string {
  let message = "";
  if (err instanceof Error) {
    message = err.message;
  } else {
    message = (err as Error).message;
  }
  return message;
}

export function showError(message: MessageApi, err: Error | unknown): void {
  message.error(formatError(err), {
    duration: 3000,
  });
}

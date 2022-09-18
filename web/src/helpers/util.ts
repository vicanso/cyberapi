import { MessageApi } from "naive-ui";

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

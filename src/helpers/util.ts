import { FormRules, MessageApi } from "naive-ui";
import dayjs from "dayjs";
import { get, has } from "lodash-es";
import { appWindow } from "@tauri-apps/api/window";
import { readText, writeText } from "@tauri-apps/api/clipboard";
import { relaunch } from "@tauri-apps/api/process";
import {
  BaseDirectory,
  writeBinaryFile,
  exists,
} from "@tauri-apps/api/fs";
import Debug from "debug";

import { appName } from "../constants/common";
import getPinYin from "./pinyin";

const debug = Debug("util");

export function isWebMode() {
  return !window.__TAURI_IPC__;
}

export async function setAppTitle(title: string) {
  if (isWebMode()) {
    return;
  }
  if (title !== appName) {
    title = `${appName} - ${title}`;
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
  return window.innerWidth || 800;
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

export async function readTextFromClipboard() {
  if (isWebMode()) {
    return navigator.clipboard.readText();
  }
  return readText();
}

export async function reload() {
  if (isWebMode()) {
    window.location.reload();
  } else {
    relaunch();
  }
}

export async function delay(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export function isJSON(data: string) {
  if (!data || data.length < 2) {
    return false;
  }
  const value = `${data[0]}${data[data.length - 1]}`;
  return value === "[]" || value === "{}";
}

export function jsonFormat(data: string) {
  try {
    const result = JSON.stringify(JSON.parse(data), null, 2);
    return result;
  } catch (err) {
    const arr = data.split("\n");
    if (arr.length < 2) {
      throw err;
    }
    // 如果第一次出错，判断是否有换行，如果有，则一行行parse
    return arr
      .map((item) => {
        if (!isJSON(item)) {
          return item;
        }
        return JSON.stringify(JSON.parse(item), null, 2);
      })
      .join("\n");
  }
}

export function convertHTTPHeaderName(name: string) {
  const arr = name.split("-");
  return arr
    .map((item) => `${item[0].toUpperCase()}${item.substring(1)}`)
    .join("-");
}

function stringToArrayBuffer(data: string): Promise<ArrayBuffer> {
  return new Promise((resolve) => {
    const b = new Blob([data]);
    const f = new FileReader();
    f.onload = (e) => {
      resolve(e.target?.result as ArrayBuffer);
    };
    f.readAsArrayBuffer(b);
  });
}

export function isMatchTextOrPinYin(content: string, keyword: string) {
  const k = keyword.toLowerCase();
  if (content.toLowerCase().includes(k)) {
    return true;
  }
  const arr = getPinYin(content);

  debug("pinyin:%s", arr.join(","));

  for (let i = 0; i < arr.length; i++) {
    if (arr[i].includes(k)) {
      return true;
    }
  }
  return false;
}

export async function writeFileToDownload(file: string, data: ArrayBuffer) {
  const arr = file.split(".");
  let baseFileName = arr[0];
  let ext = "";
  if (arr.length >= 2) {
    baseFileName = arr.slice(0, arr.length - 1).join(".");
    ext = `.${arr[arr.length - 1]}`;
  }
  const opt = {
    dir: BaseDirectory.Download,
  };
  // 如果有重名的，则数字+1
  for (let i = 0; i < 10; i++) {
    const file = (i === 0 ? baseFileName : `${baseFileName}-${i}`) + ext;
    const fileExists = await exists(file, opt);
    // eslint-disable-next-line @typescript-eslint/ban-ts-comment
    // @ts-ignore
    if (!fileExists) {
      await writeBinaryFile(file, data, opt);
      return;
    }
  }
  throw new Error(`file(${file}) exist`);
}

export async function writeSettingToDownload(arr: unknown, name: string) {
  const data = JSON.stringify(arr, null, 2);
  const buf = await stringToArrayBuffer(data);
  await writeFileToDownload(`cyberapi-${name}.json`, buf);
}

import {
  BaseDirectory,
  readBinaryFile,
  readTextFile,
} from "@tauri-apps/api/fs";
import { open } from "@tauri-apps/api/dialog";
import { toString, get, trim } from "lodash-es";
import { fromUint8Array } from "js-base64";
import { getLatestResponse, getResponseBody } from "./http_response";
interface FnHandler {
  // 原始字符
  text: string;
  // 函数列表
  fnList: string[];
  // 初始参数
  param: string | string[];
}

enum Fn {
  readTextFile = "readTextFile",
  rtf = "rtf",
  readFile = "readFile",
  rf = "rf",
  base64 = "base64",
  b64 = "b64",
  openFile = "openFile",
  of = "of",
  get = "get",
  g = "g",
  timestamp = "timestamp",
  ts = "ts",
}

function trimParam(param: string): string | string[] {
  const arr = param.split(",").map((item) => {
    item = item.trim();
    item = trim(item, "'");
    item = trim(item, '"');
    return item;
  });

  // 单引号替换为双引号
  // const str = `[${param.replaceAll("'", '"')}]`;
  // const arr = JSON.parse(str);
  if (arr.length < 2) {
    return arr[0];
  }
  return arr;
}

export function parseFunctions(value: string): FnHandler[] {
  const reg = /\{\{([\s\S]+?)\}\}/g;
  const parmaReg = /\(([\s\S]*?)\)/;
  let result: RegExpExecArray | null;
  const handlers: FnHandler[] = [];
  while ((result = reg.exec(value)) !== null) {
    if (result.length !== 2) {
      break;
    }
    const paramResult = parmaReg.exec(result[1]);
    if (paramResult?.length !== 2) {
      break;
    }
    const fnList = result[1].replace(paramResult[0], "").split(".");
    handlers.push({
      text: result[0],
      fnList: fnList,
      param: trimParam(paramResult[1]),
    });
  }
  return handlers;
}

export async function doFnHandler(handler: FnHandler): Promise<string> {
  const { param, fnList } = handler;
  let p: unknown = param;
  const size = fnList.length;
  //   函数处理从后往前
  for (let index = size - 1; index >= 0; index--) {
    const fn = fnList[index];
    switch (fn) {
      case Fn.readTextFile:
      case Fn.rtf:
        {
          p = await readTextFile(toString(p), {
            dir: BaseDirectory.Download,
          });
        }
        break;
      case Fn.readFile:
      case Fn.rf:
        {
          p = await readBinaryFile(toString(p), {
            dir: BaseDirectory.Download,
          });
        }
        break;
      case Fn.base64:
      case Fn.b64:
        {
          p = fromUint8Array(p as Uint8Array);
        }
        break;
      case Fn.openFile:
      case Fn.of:
        {
          const selected = await open();
          if (selected) {
            p = selected as string;
          }
        }
        break;
      case Fn.timestamp:
      case Fn.ts:
        {
          p = `${Math.round(Date.now() / 1000)}`;
        }
        break;
      case Fn.get:
      case Fn.g:
        {
          const arr = toString(p).split(",");
          if (arr.length !== 2) {
            throw new Error("params of get from response is invalid");
          }
          const resp = await getLatestResponse(arr[0].trim());
          if (resp) {
            const result = getResponseBody(resp);
            p = get(result.json, arr[1].trim());
          }
        }
        break;
      default:
        break;
    }
  }
  return toString(p);
}

import {
  BaseDirectory,
  readBinaryFile,
  readTextFile,
} from "@tauri-apps/api/fs";
import { open } from "@tauri-apps/api/dialog";
import { trim, toString } from "lodash-es";
import { fromUint8Array } from "js-base64";
interface FnHandler {
  // 原始字符
  text: string;
  // 函数列表
  fnList: string[];
  // 初始参数
  param: string;
}

enum Fn {
  readTextFile = "readTextFile",
  readFile = "readFile",
  base64 = "base64",
  openFile = "openFile",
  timestamp = "timestamp",
}

function trimParam(param: string) {
  param = param.trim();
  param = trim(param, "'");
  param = trim(param, '"');
  return param;
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
        {
          p = await readTextFile(toString(p), {
            dir: BaseDirectory.Download,
          });
        }
        break;
      case Fn.readFile:
        {
          p = await readBinaryFile(toString(p), {
            dir: BaseDirectory.Download,
          });
        }
        break;
      case Fn.base64:
        {
          p = fromUint8Array(p as Uint8Array);
        }
        break;
      case Fn.openFile:
        {
          const selected = await open();
          if (selected) {
            p = selected as string;
          }
        }
        break;
      case Fn.timestamp:
        {
          p = `${Math.round(Date.now() / 1000)}`;
        }
        break;
      default:
        break;
    }
  }
  return toString(p);
}

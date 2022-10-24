import { decode } from "js-base64";
import dayjs from "dayjs";
import { forEach } from "lodash-es";
import { getLatestResponseStore } from "../stores/local";
import { HTTPRequest } from "./http_request";

const applicationJSON = "application/json";

export interface HTTPStats {
  remoteAddr: string;
  dnsLookup: number;
  connect: number;
  serverProcessing: number;
  contentTransfer: number;
  total: number;
}

export interface HTTPResponse {
  [key: string]: unknown;
  // api id
  api: string;
  req: HTTPRequest;
  // 原始body的大小(未解压)
  bodySize: number;
  // 耗时(ms)
  latency: number;
  status: number;
  headers: Map<string, string[]>;
  body: string;
  stats: HTTPStats;
}

export enum ResponseBodyCategory {
  JSON = "json",
  Binary = "binary",
  Text = "text",
}
export interface ResponseBodyResult {
  category: ResponseBodyCategory;
  data: string;
  size: number;
  json?: Map<string, unknown>;
}

const statusTextMap = new Map<string, string>();
(() => {
  const dict = {
    100: "Continue",
    101: "Switching Protocols",
    102: "Processing",
    103: "Early Hints",
    200: "OK",
    201: "Created",
    202: "Accepted",
    203: "Non-Authoritative Information",
    204: "No Content",
    205: "Reset Content",
    206: "Partial Content",
    207: "Multi-Status",
    208: "Already Reported",
    226: "IM Used",
    300: "Multiple Choices",
    301: "Moved Permanently",
    302: "Found",
    303: "See Other",
    304: "Not Modified",
    305: "Use Proxy",
    307: "Temporary Redirect",
    308: "Permanent Redirect",
    400: "Bad Request",
    401: "Unauthorized",
    402: "Payment Required",
    403: "Forbidden",
    404: "Not Found",
    405: "Method Not Allowed",
    406: "Not Acceptable",
    407: "Proxy Authentication Required",
    408: "Request Timeout",
    409: "Conflict",
    410: "Gone",
    411: "Length Required",
    412: "Precondition Failed",
    413: "Request Entity Too Large",
    414: "Request URI Too Long",
    415: "Unsupported Media Type",
    416: "Requested Range Not Satisfiable",
    417: "Expectation Failed",
    418: "I'm a teapot",
    421: "Misdirected Request",
    422: "Unprocessable Entity",
    423: "Locked",
    424: "Failed Dependency",
    425: "Too Early",
    426: "Upgrade Required",
    428: "Precondition Required",
    429: "Too Many Requests",
    431: "Request Header Fields Too Large",
    451: "Unavailable For Legal Reasons",
    500: "Internal Server Error",
    501: "Not Implemented",
    502: "Bad Gateway",
    503: "Service Unavailable",
    504: "Gateway Timeout",
    505: "HTTP Version Not Supported",
    506: "Variant Also Negotiates",
    507: "Insufficient Storage",
    508: "Loop Detected",
    510: "Not Extended",
    511: "Network Authentication Required",
  };
  forEach(dict, (value, key) => {
    statusTextMap.set(key.toString(), value);
  });
})();

export function getStatusText(code: number) {
  return statusTextMap.get(code.toString()) || "";
}

export function getResponseBody(resp: HTTPResponse): ResponseBodyResult {
  const { headers, body } = resp;
  let category = ResponseBodyCategory.Binary;
  let data = body;
  let size = -1;
  let json: Map<string, unknown> = new Map<string, unknown>();
  let isJSON = false;
  headers.forEach((values, key) => {
    const k = key.toLowerCase();
    switch (k) {
      case "content-type":
        {
          const value = values.join(" ");
          if (value.includes(applicationJSON)) {
            category = ResponseBodyCategory.JSON;
            data = decode(data);
            json = JSON.parse(data);
            isJSON = true;
            // format
            data = JSON.stringify(json, null, 2);
          } else if (value.includes("text")) {
            category = ResponseBodyCategory.Text;
            data = decode(data);
          }
        }
        break;
      case "content-length":
        {
          const v = Number.parseInt(values[0]);
          if (!Number.isNaN(v)) {
            size = v;
          }
        }
        break;
    }
  });
  if (size < 0) {
    size = Math.ceil((body.length / 4) * 3);
  }

  const result: ResponseBodyResult = {
    category,
    data,
    size,
  };
  if (isJSON) {
    result.json = json;
  }
  return result;
}

// 缓存的response数据
interface Response {
  resp: HTTPResponse;
  createdAt: string;
}

const limit = 10;

// TODO 如何定期清除相关数据

export async function addLatestResponse(resp: HTTPResponse) {
  const id = resp.api;
  const store = getLatestResponseStore();
  if (!id || !store) {
    return;
  }
  const arr = (await store.getItem<Response[]>(id)) || [];
  if (arr.length >= limit) {
    arr.pop();
  }
  // 添加至顶部
  arr.unshift({
    resp,
    createdAt: dayjs().format(),
  });
  await store.setItem(id, arr);
}

export async function getLatestResponse(id: string) {
  const arr = (await getLatestResponseStore().getItem<Response[]>(id)) || [];
  const result = arr.find((item) => item.resp.api === id);
  if (result) {
    return result.resp;
  }
}

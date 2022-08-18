import { forEach, isArray } from "lodash-es";
import { decode } from "js-base64";

import { run, cmdDoHTTPRequest } from "./invoke";
import { KVParam } from "./interface";
import { isWebMode } from "../helpers/util";
import dayjs from "dayjs";

export enum HTTPMethod {
  GET = "GET",
  POST = "POST",
  PUT = "PUT",
  PATCH = "PATCH",
  DELETE = "DELETE",
  OPTIONS = "OPTIONS",
  HEAD = "HEAD",
}

export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body: string;
  contentType: string;
  headers: KVParam[];
  query: KVParam[];
}

export function convertRequestToCURL(req: HTTPRequest) {
  const queryList: string[] = [];
  req.query?.forEach((kv) => {
    if (!kv.enabled) {
      return;
    }
    queryList.push(`${kv.key}=${encodeURIComponent(kv.value)}`);
  });
  let uri = req.uri;
  if (uri.includes("?")) {
    uri += `&${queryList.join("&")}`;
  } else {
    uri += `?${queryList.join("&")}`;
  }
  const headerList: string[] = [];
  let includeContentType = false;
  req.headers?.forEach((kv) => {
    if (!kv.enabled) {
      return;
    }
    if (kv.key.toLowerCase() === "content-type") {
      includeContentType = true;
    }
    headerList.push(`-H '${kv.key}:${kv.value}'`);
  });
  if (!includeContentType && req.contentType) {
    headerList.push(`-H 'Content-Type:${req.contentType}'`);
  }
  // TODO body
  let body = "";
  if (req.body) {
    const json = JSON.stringify(JSON.parse(req.body));
    body = ` -d '${json}' `;
  }
  const method = req.method || "GET";
  return `curl -X${method.toUpperCase()}${body}${headerList.join(
    " "
  )} '${uri}'`;
}

export interface HTTPResponse {
  [key: string]: unknown;
  // api id
  api: string;
  // 耗时(ms)
  latency: number;
  status: number;
  headers: Map<string, string[]>;
  body: string;
}

const applicationJSON = "application/json";

export enum ResponseBodyCategory {
  JSON = "json",
  Binary = "binary",
  Text = "text",
}
export interface ResponseBodyResult {
  category: ResponseBodyCategory;
  data: string;
  size: number;
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
  headers.forEach((values, key) => {
    const k = key.toLowerCase();
    switch (k) {
      case "content-type":
        {
          const value = values.join(" ");
          if (value.includes(applicationJSON)) {
            category = ResponseBodyCategory.JSON;
            data = decode(data);
            // format
            data = JSON.stringify(JSON.parse(data), null, 2);
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

  return {
    category,
    data,
    size,
  };
}

interface Response {
  resp: HTTPResponse;
  createdAt: string;
}

const latestResponseList: Response[] = [];

function addLatestResponse(resp: HTTPResponse) {
  // 添加至顶部
  latestResponseList.unshift({
    resp,
    createdAt: dayjs().format(),
  });
  if (latestResponseList.length > 10) {
    latestResponseList.pop();
  }
}

export function getLatestResponse(id: string) {
  const result = latestResponseList.find((item) => item.resp.api === id);
  if (result) {
    return result.resp;
  }
}

export async function doHTTPRequest(
  id: string,
  req: HTTPRequest
): Promise<HTTPResponse> {
  if (!req.headers) {
    req.headers = [];
  }
  if (!req.query) {
    req.query = [];
  }
  const method = req.method || HTTPMethod.GET;
  let body = req.body || "";
  let contentType = req.contentType || "";
  // 非此类请求，将body设置为空
  if (
    ![HTTPMethod.POST, HTTPMethod.PATCH, HTTPMethod.PUT].includes(
      method as HTTPMethod
    )
  ) {
    body = "";
    contentType = "";
  }
  const params = {
    method: method,
    uri: req.uri,
    body,
    contentType,
    headers: req.headers,
    query: req.query,
  };
  if (isWebMode()) {
    const headers = new Map<string, string[]>();
    headers.set("Content-Type", [applicationJSON]);
    const resp = {
      api: id,
      latency: 1860,
      status: 200,
      headers,
      body: window.btoa(JSON.stringify(params)),
    };

    addLatestResponse(resp);
    return Promise.resolve(resp);
  }

  const resp = await run<HTTPResponse>(cmdDoHTTPRequest, {
    req: params,
    api: id,
  });
  if (resp.latency <= 0) {
    resp.latency = 1;
  }
  // 转换为Map<string, string[]>
  const headers = new Map<string, string[]>();
  forEach(resp.headers, (value, key) => {
    const k = key.toString();
    if (isArray(value)) {
      headers.set(k, value);
    } else {
      headers.set(k, [value as string]);
    }
  });
  resp.headers = headers;
  addLatestResponse(resp);
  return resp;
}

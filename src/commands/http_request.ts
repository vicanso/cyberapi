import { forEach, isArray } from "lodash-es";
import { encode } from "js-base64";
import { ulid } from "ulid";
import { getVersion, getTauriVersion } from "@tauri-apps/api/app";
import { arch, type, version } from "@tauri-apps/api/os";

import { run, cmdDoHTTPRequest } from "./invoke";
import { KVParam } from "./interface";
import { isWebMode, delay } from "../helpers/util";
import { doFnHandler, parseFunctions } from "./fn";
import { HTTPResponse, addLatestResponse } from "./http_response";

export enum HTTPMethod {
  GET = "GET",
  POST = "POST",
  PUT = "PUT",
  PATCH = "PATCH",
  DELETE = "DELETE",
  OPTIONS = "OPTIONS",
  HEAD = "HEAD",
}

export enum ContentType {
  JSON = "application/json",
  Form = "application/x-www-form-urlencoded",
  Multipart = "multipart/form-data",
  XML = "application/xml",
  Plain = "text/plain",
}

export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body: string;
  contentType: string;
  headers: KVParam[];
  query: KVParam[];
  auth: KVParam[];
}

function convertKVListToURLValues(kvList: KVParam[]) {
  if (!kvList || kvList.length === 0) {
    return [];
  }
  const arr: string[] = [];
  kvList.forEach((kv) => {
    if (!kv.enabled) {
      return;
    }
    arr.push(`${kv.key}=${encodeURIComponent(kv.value)}`);
  });
  return arr;
}

export async function convertRequestToCURL(
  collection: string,
  req: HTTPRequest
) {
  await convertKVParams(collection, req.query);
  await convertKVParams(collection, req.headers);
  const queryList = convertKVListToURLValues(req.query);

  let uri = req.uri;
  if (queryList.length !== 0) {
    if (uri.includes("?")) {
      uri += `&${queryList.join("&")}`;
    } else {
      uri += `?${queryList.join("&")}`;
    }
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
  let body = " ";
  if (req.body) {
    body = await convertBody(collection, req.body);
    switch (req.contentType) {
      case ContentType.JSON:
        body = JSON.stringify(JSON.parse(body));
        break;
      case ContentType.Form:
        {
          const arr: KVParam[] = JSON.parse(body);
          body = convertKVListToURLValues(arr).join("&");
        }
        break;
      default:
        break;
    }
    body = ` -d '${body}' `;
  }
  const method = req.method || "GET";
  return `curl -X${method.toUpperCase()}${body}${headerList.join(
    " "
  )} '${uri}'`;
}

async function convertBody(collection: string, body: string) {
  const handlers = parseFunctions(collection, body);
  if (handlers.length === 0) {
    return body;
  }
  for (let i = 0; i < handlers.length; i++) {
    const handler = handlers[i];
    const result = await doFnHandler(handler);
    // 替换result的内容
    body = body.replace(handler.text, result);
  }
  return body;
}

export async function convertKVParams(collection: string, params: KVParam[]) {
  if (!params || params.length === 0) {
    return;
  }
  for (let i = 0; i < params.length; i++) {
    const param = params[i];
    const handlers = parseFunctions(collection, param.value);
    if (handlers.length === 0) {
      continue;
    }
    let { value } = param;
    for (let j = 0; j < handlers.length; j++) {
      const handler = handlers[j];
      const result = await doFnHandler(handler);
      // 替换result的内容
      value = value.replace(handler.text, result);
    }
    param.value = value;
  }
}

export const abortRequestID = ulid();

// Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)
let userAgent = "";

export async function doHTTPRequest(
  id: string,
  collection: string,
  req: HTTPRequest
): Promise<HTTPResponse> {
  if (!req.headers) {
    req.headers = [];
  }
  if (!req.query) {
    req.query = [];
  }
  if (!req.auth) {
    req.auth = [];
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
  body = await convertBody(collection, body);
  // 如果是form
  if (body && contentType === ContentType.Form) {
    const arr = JSON.parse(body) as KVParam[];
    const result: string[] = [];
    arr.forEach((item) => {
      if (!item.enabled) {
        return;
      }
      result.push(
        `${window.encodeURIComponent(item.key)}=${window.encodeURIComponent(
          item.value
        )}`
      );
    });
    body = result.join("&");
  }
  if (body && contentType === ContentType.Multipart) {
    const arr = JSON.parse(body) as KVParam[];
    const result: string[] = [];
    arr.forEach((item) => {
      if (!item.enabled) {
        return;
      }
      result.push(
        `${window.encodeURIComponent(item.key)}=${window.encodeURIComponent(
          item.value
        )}`
      );
    });
    body = result.join("&");
  }
  const params = {
    method: method,
    uri: req.uri,
    body,
    contentType,
    headers: req.headers,
    query: req.query,
  };
  await convertKVParams(collection, params.query);
  await convertKVParams(collection, params.headers);
  if (isWebMode()) {
    const ms = Math.random() * 2000;
    await delay(ms);
    const headers = new Map<string, string[]>();
    headers.set("Content-Type", ["application/json"]);
    const resp = {
      api: id,
      req: req,
      latency: Math.ceil(ms),
      status: 200,
      bodySize: 0,
      headers,
      body: encode(JSON.stringify(params)),
      stats: {
        remoteAddr: "127.0.0.1:80",
        gotFirstResponseByte: 20,
        done: 30,
      },
    };

    addLatestResponse(resp);
    return Promise.resolve(resp);
  }

  if (!userAgent) {
    const appVersion = await getVersion();
    const appOS = await type();
    const appOSVersion = await version();
    const appArch = await arch();
    const tauriVersion = await getTauriVersion();
    userAgent = `CyberAPI/${appVersion} (${appOS}; tauri/${tauriVersion}; ${appOSVersion}; ${appArch})`;
  }

  params.headers.push({
    key: "User-Agent",
    value: userAgent,
    enabled: true,
  });

  const auth = req.auth.filter((item) => item.enabled);
  if (auth.length) {
    const value = encode(`${auth[0].key}:${auth[0].value}`);
    params.headers.push({
      key: "Authorization",
      value: `Basic ${value}`,
      enabled: true,
    });
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

  resp.req = req;
  resp.headers = headers;
  addLatestResponse(resp);
  return resp;
}

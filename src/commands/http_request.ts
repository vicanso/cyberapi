import { forEach, isArray } from "lodash-es";
import { encode } from "js-base64";
import { ulid } from "ulid";

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

export async function convertRequestToCURL(req: HTTPRequest) {
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
    switch (req.contentType) {
      case ContentType.JSON:
        body = JSON.stringify(JSON.parse(req.body));
        break;
      case ContentType.Form:
        {
          const arr: KVParam[] = JSON.parse(req.body);
          body = convertKVListToURLValues(arr).join("&");
        }
        break;
      default:
        break;
    }
    body = await convertBody(body);
    body = ` -d '${body}' `;
  }
  const method = req.method || "GET";
  return `curl -X${method.toUpperCase()}${body}${headerList.join(
    " "
  )} '${uri}'`;
}

async function convertBody(body: string) {
  const handlers = parseFunctions(body);
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

export async function convertKVParams(params: KVParam[]) {
  for (let i = 0; i < params.length; i++) {
    const param = params[i];
    const handlers = parseFunctions(param.value);
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
  await convertKVParams(params.query);
  await convertKVParams(params.headers);
  params.body = await convertBody(params.body);
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
      headers,
      body: encode(JSON.stringify(params)),
      stats: {
        remoteAddr: "",
      },
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
  resp.req = req;
  resp.headers = headers;
  addLatestResponse(resp);
  return resp;
}

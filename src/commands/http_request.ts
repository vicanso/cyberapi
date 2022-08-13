import { forEach, isArray } from "lodash-es";

import { run, cmdDoHTTPRequest } from "./invoke";
import { KVParam } from "./interface";
import { isWebMode } from "../helpers/util";

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

export interface HTTPResponse {
  [key: string]: unknown;
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
}

export function getResponseBody(resp: HTTPResponse): ResponseBodyResult {
  const { headers, body } = resp;
  let category = ResponseBodyCategory.Binary;
  let data = body;
  headers.forEach((values, key) => {
    if (key.toLowerCase() !== "content-type") {
      return;
    }
    const value = values.join(" ");
    if (value.includes(applicationJSON)) {
      category = ResponseBodyCategory.JSON;
      data = window.atob(data);
    } else if (value.includes("text")) {
      category = ResponseBodyCategory.Text;
      data = window.atob(data);
    }
  });

  return {
    category,
    data,
  };
}

export async function doHTTPRequest(req: HTTPRequest): Promise<HTTPResponse> {
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
    return Promise.resolve({
      status: 200,
      headers,
      body: window.btoa(JSON.stringify(params)),
    });
  }

  const resp = await run<HTTPResponse>(cmdDoHTTPRequest, {
    req: params,
  });
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
  return resp;
}

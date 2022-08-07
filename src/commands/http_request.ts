import { run, cmdDoHTTPRequest } from "./invoke";

export enum HTTPMethod {
  GET = "GET",
  POST = "POST",
  PUT = "PUT",
  PATCH = "PATCH",
  DELETE = "DELETE",
  OPTIONS = "OPTIONS",
  HEAD = "HEAD",
}

export interface HTTPRequestKVParam {
  [key: string]: unknown;
  key: string;
  value: string;
  enabled: boolean;
}
export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body: string;
  contentType: string;
  headers: HTTPRequestKVParam[];
  query: HTTPRequestKVParam[];
}

export interface HTTPResponse {
  [key: string]: unknown;
  status: number;
  headers: Map<string, string[]>;
  body: string;
}

export async function doHTTPRequest(req: HTTPRequest): Promise<HTTPResponse> {
  if (!req.headers) {
    req.headers = [];
  }
  if (!req.query) {
    req.query = [];
  }
  return await run<HTTPResponse>(cmdDoHTTPRequest, {
    // Map无法转换，因此做一次转换
    req: {
      method: req.method,
      uri: req.uri,
      body: req.body || "",
      contentType: req.contentType,
      headers: req.headers,
      query: req.query,
    },
  });
}

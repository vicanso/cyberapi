import { run, cmdDoHTTPRequest } from "./invoke";

export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body?: string;
  headers?: Map<string, string[]>;
  query?: Map<string, string[]>;
}

export interface HTTPResponse {
  [key: string]: unknown;
  status: number;
  headers: Map<string, string[]>;
  body: string;
}

export async function doHTTPRequest(req: HTTPRequest): Promise<HTTPResponse> {
  if (!req.headers) {
    req.headers = new Map<string, string[]>();
  }
  if (!req.query) {
    req.query = new Map<string, string[]>();
  }
  return await run<HTTPResponse>(cmdDoHTTPRequest, {
    // Map无法转换，因此做一次转换
    req: {
      method: req.method,
      uri: req.uri,
      body: req.body || "",
      headers: Object.fromEntries(req.headers),
      query: Object.fromEntries(req.query),
    },
  });
}

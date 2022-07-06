import { run, cmdDoHTTPRequest } from "./invoke";

export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body: string;
  headers: Map<string, string[]>;
}

export interface HTTPResponse {
  [key: string]: unknown;
  status: number;
  headers: Map<string, string[]>;
  body: string;
}

export async function doHTTPRequest(httpRequest: HTTPRequest): Promise<HTTPResponse> {
  return await run<HTTPResponse>(cmdDoHTTPRequest, {
    httpRequest,
  });
}

import { run, cmdDoHTTPRequest } from "./invoke";

export interface HTTPRequest {
  [key: string]: unknown;
  method: string;
  uri: string;
  body: string;
  headers: Map<string, string[]>;
}

export async function doHTTPRequest(httpRequest: HTTPRequest) {
  await run(cmdDoHTTPRequest, {
    httpRequest,
  });
}

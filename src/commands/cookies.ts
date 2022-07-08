import { get, values } from "lodash-es";
import { cmdListCookie, run } from "./invoke";

export interface Cookie {
  [key: string]: unknown;
  name: string;
  value: string;
  path: string;
  domain: string;
  expires: string;
}

export async function listCookie(): Promise<Cookie[]> {
  const arr = await run<string[]>(cmdListCookie, {});
  const cookies:Cookie[] = [];
  arr.forEach((data) => {
    const item = JSON.parse(data);
    const cookie = get(item, "raw_cookie");
    if (!cookie) {
      return;
    }
    const cookieValues = (cookie as string).split(";");
    const [name, value] = cookieValues[0].split("=");
    const path = (get(item, "path.0") || "/") as string;

    const domainValues = values(get(item, "domain"));
    let domain = "";
    if (domainValues && domainValues.length) {
      domain = domainValues[0];
    }
    const expires = get(item, "expires.AtUtc") as string;
    cookies.push({
      name,
      value,
      path,
      domain,
      expires,
    });
  });
  return cookies;
}

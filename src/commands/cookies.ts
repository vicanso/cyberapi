import { get, values } from "lodash-es";
import { isWebMode } from "../helpers/util";
import {
  cmdAddCookie,
  cmdClearCookie,
  cmdDeleteCookie,
  cmdListCookie,
  run,
} from "./invoke";

export interface Cookie {
  [key: string]: unknown;
  name: string;
  value: string;
  path: string;
  domain: string;
  expires: string;
}

export async function listCookie(): Promise<Cookie[]> {
  if (isWebMode()) {
    return Promise.resolve([
      {
        name: "cybertect",
        value: "CBBVJIUT8Q9EEFDKF9H0",
        path: "/",
        domain: "cybertect.npmtrend.com",
        expires: "Wed, 31 Jan 2024 16:00:00 GMT",
      },
      {
        name: "cybertect.sig",
        value: "iIoKqqpgXc-Ao-ilTf4XdaNyblsdKauy0fVqISbikoU",
        path: "/",
        domain: "",
        expires: "Wed, 31 Jan 2024 16:00:00 GMT",
      },
    ]);
  }
  const arr = await run<string[]>(cmdListCookie, {});
  if (!arr || !arr.length) {
    return [];
  }
  const cookies: Cookie[] = [];
  arr.forEach((data) => {
    const item = JSON.parse(data);
    const cookie = get(item, "raw_cookie");
    if (!cookie) {
      return;
    }
    const cookieValues = (cookie as string).split(";");
    const [name, ...value] = cookieValues[0].split("=");
    const path = (get(item, "path.0") || "/") as string;

    const domainValues = values(get(item, "domain"));
    let domain = "";
    if (domainValues && domainValues.length) {
      domain = domainValues[0];
    }
    const expires = get(item, "expires.AtUtc") as string;
    cookies.push({
      name,
      value: value.join("="),
      path: path || "",
      domain: domain || "",
      expires: expires || "",
    });
  });
  return cookies;
}

export async function deleteCookie(c: Cookie) {
  if (isWebMode()) {
    return;
  }
  await run(cmdDeleteCookie, {
    c,
  });
}

export async function clearCookie() {
  if (isWebMode()) {
    return;
  }
  await run(cmdClearCookie, {});
}

export async function addOrUpdate(c: Cookie) {
  if (isWebMode()) {
    return;
  }
  await run(cmdAddCookie, {
    c,
  });
}

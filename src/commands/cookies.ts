import { cmdListCookie, run } from "./invoke";

export async function listCookie() {
  return await run(cmdListCookie, {});
}

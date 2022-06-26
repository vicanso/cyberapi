import { run } from "./invoke";

interface APISetting {
  name: string;
}

export async function saveAPISetting(setting: APISetting) {
  try {
    const resp = await run("save_api", {
      setting,
    });
    console.dir(resp);
  } catch (err) {
    console.error(err);
  }
}

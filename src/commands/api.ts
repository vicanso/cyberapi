import { run } from "./invoke";

interface HTTPSetting {
  method?: string;
  url?: string;
  query?: Record<string, string[]>;
  header?: Record<string, string[]>;
  body?: {
    dataType: string;
    data?: string;
  };
  // TODO AUTH
}
interface APISetting {
  id: string;
  // 名称
  name?: string;
  // 路径
  path?: string;
  // HTTP配置
  http?: HTTPSetting;
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

import { v4 as uuidv4 } from 'uuid';

import { run } from "./invoke";


interface APISetting {
  id: string;
  // 名称
  name: string;
  // 目录ID
  path: string;
   // 类型(http, graphQL)
    category: string;
   // 配置信息
    setting: string;
   // 创建时间
    createdAt: string;
   // 更新时间
    updatedAt: string;
}


export async function createAPISetting(): Promise<string> {
  const id = uuidv4();
  await run("add_api_setting", {
    id, 
  });
  return id; 
}

export async function listAPISetting(): Promise<APISetting[]> {
  const resp = await run<APISetting[]>("list_api_setting"); 
  return resp;
}

export async function updateAPISetting(setting: APISetting) {
  try {
    const resp = await run("save_api", {
      setting,
    });
    console.dir(resp);
  } catch (err) {
    console.error(err);
  }
}

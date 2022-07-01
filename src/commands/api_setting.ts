import { v4 as uuidv4 } from "uuid";
import localforage from "localforage";

import { isWebMode } from "../helpers/util";
import {
  run,
  cmdAddAPISetting,
  cmdListAPISetting,
  cmdUpdateAPISetting,
} from "./invoke";

var apiStore = localforage.createInstance({
  name: "apiSettings",
});

export interface APISetting {
  [key: string]: unknown;
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

async function fakeListAPISetting(): Promise<APISetting[]> {
  const result = await apiStore.getItem<APISetting[]>("fake");
  if (result != null) {
    return result;
  }
  return [];
}
async function fakeAddAPISetting(setting: APISetting) {
  const result = await fakeListAPISetting();
  result.push(setting);
  await apiStore.setItem("fake", result);
}

async function fakeUpdateAPISetting(setting: APISetting) {
  const result = await fakeListAPISetting();
  let found = -1;
  result.forEach((item, index) => {
    if (item.id == setting.id) {
      found = index;
    }
  });
  if (found !== -1) {
    result[found] = setting;
  }
  await apiStore.setItem("fake", result);
}

export async function createAPISetting(): Promise<string> {
  const id = uuidv4();
  if (isWebMode()) {
    await fakeAddAPISetting({
      id,
      name: "",
      path: "",
      category: "",
      setting: "",
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    });
  } else {
    await run(cmdAddAPISetting, {
      id,
    });
  }
  return id;
}

export async function listAPISetting(): Promise<APISetting[]> {
  if (isWebMode()) {
    return await fakeListAPISetting();
  }
  return await run<APISetting[]>(cmdListAPISetting);
}

export async function updateAPISetting(setting: APISetting) {
  if (isWebMode()) {
    return await fakeUpdateAPISetting(setting);
  }
  await run(cmdUpdateAPISetting, {
    setting,
  });
}

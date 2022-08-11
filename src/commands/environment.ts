import { ulid } from "ulid";
import dayjs from "dayjs";
import { isWebMode } from "../helpers/util";
import {
  cmdAddEnvironment,
  cmdDeleteEnvironment,
  cmdListEnvironment,
  cmdUpdateEnvironment,
  run,
} from "./invoke";
import { fakeList, fakeAdd, fakeUpdate, fakeUpdateStore } from "./fake";
import { APISetting } from "./api_setting";

const store = "environments";

export enum EnvironmentStatus {
  Enabled = "1",
  Disabled = "0",
}

export interface Environment {
  [key: string]: unknown;
  id: string;
  collection: string;
  // 名称
  name: string;
  // 值
  value: string;
  // 是否启用(0:禁用 1:启用)
  enabled: string;
  // 创建时间
  createdAt: string;
  // 更新时间
  updatedAt: string;
}

export function newDefaultEnvironment(): Environment {
  const id = ulid();
  return {
    id,
    collection: "",
    name: "",
    value: "",
    enabled: EnvironmentStatus.Enabled,
    createdAt: dayjs().format(),
    updatedAt: dayjs().format(),
  };
}

export async function createEnvironment(env: Environment) {
  if (isWebMode()) {
    await fakeAdd<Environment>(store, env);
    return;
  }
  await run(cmdAddEnvironment, {
    env,
  });
}

export async function listEnvironment(
  collection: string
): Promise<Environment[]> {
  if (isWebMode()) {
    return await fakeList<Environment>(store);
  }
  return await run<Environment[]>(cmdListEnvironment, {
    collection,
  });
}

export async function updateEnvironment(env: Environment) {
  if (isWebMode()) {
    await fakeUpdate(store, env);
    return;
  }
  await run(cmdUpdateEnvironment, {
    env,
  });
}

export async function deleteEnvironment(ids: string[]) {
  if (isWebMode()) {
    const arr = await fakeList<APISetting>(store);
    const result = arr.filter((item) => {
      return !ids.includes(item.id);
    });
    await fakeUpdateStore(store, result);
  }
  await run(cmdDeleteEnvironment, {
    ids,
  });
}

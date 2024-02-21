import { ulid } from "ulid";
import dayjs from "dayjs";
import { isWebMode } from "../helpers/util";
import {
  cmdAddVariable,
  cmdDeleteVariable,
  cmdListVariable,
  cmdUpdateVariable,
  run,
} from "./invoke";
import { fakeList, fakeAdd, fakeUpdate, fakeUpdateStore } from "./fake";

const store = "variables";

export enum VariableStatus {
  Enabled = "1",
  Disabled = "0",
}

export enum VariableCategory {
  // 环境变量
  Environment = "env",
  // 自定义
  Customize = "customize",
  // 全局请求头
  GlobalReqHeaders = "globalReqHeaders",
}

export interface Variable {
  [key: string]: unknown;
  id: string;
  category: string;
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

export function newDefaultVariable(): Variable {
  const id = ulid();
  return {
    id,
    category: "",
    collection: "",
    name: "",
    value: "",
    enabled: VariableStatus.Enabled,
    createdAt: dayjs().format(),
    updatedAt: dayjs().format(),
  };
}

export async function createVariable(value: Variable) {
  if (isWebMode()) {
    await fakeAdd<Variable>(store, value);
  }
  await run(cmdAddVariable, {
    value,
  });
}

export async function listVariable(
  collection: string,
  category: string,
): Promise<Variable[]> {
  if (isWebMode()) {
    return await fakeList<Variable>(store);
  }
  return await run<Variable[]>(cmdListVariable, {
    collection,
    category,
  });
}

export async function updateVariable(value: Variable) {
  if (isWebMode()) {
    return await fakeUpdate(store, value);
    return;
  }
  await run(cmdUpdateVariable, {
    value,
  });
}

export async function deleteVariable(ids: string[]) {
  if (isWebMode()) {
    const arr = await fakeList<Variable>(store);
    const result = arr.filter((item) => {
      return !ids.includes(item.id);
    });
    await fakeUpdateStore(store, result);
  }
  await run(cmdDeleteVariable, {
    ids,
  });
}

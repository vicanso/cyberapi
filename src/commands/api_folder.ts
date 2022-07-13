import dayjs from "dayjs";
import { ulid } from "ulid";

import { isWebMode } from "../helpers/util";
import {
  run,
  cmdAddAPIFolder,
  cmdListAPIFolder,
  cmdUpdateAPIFolder,
} from "./invoke";
import { fakeList, fakeAdd, fakeUpdate } from "./fake";

const store = "apiFolders";

export interface APIFolder {
  [key: string]: unknown;
  id: string;
  collection: string;
  children: string;
  // 名称
  name: string;
  // 创建时间
  createdAt: string;
  // 更新时间
  updatedAt: string;
}

export function newDefaultAPIFolder(): APIFolder {
  const id = ulid();
  return {
    id,
    collection: "",
    children: "",
    name: "",
    createdAt: dayjs().format(),
    updatedAt: dayjs().format(),
  };
}

export async function createAPIFolder(folder: APIFolder): Promise<void> {
  if (isWebMode()) {
    await fakeAdd<APIFolder>(store, folder);
    return;
  }
  await run(cmdAddAPIFolder, {
    folder,
  });
}

export async function listAPIFolder(): Promise<APIFolder[]> {
  if (isWebMode()) {
    return await fakeList<APIFolder>(store);
  }
  return await run<APIFolder[]>(cmdListAPIFolder);
}

export async function updateAPIFolder(folder: APIFolder) {
  if (isWebMode()) {
    return await fakeUpdate(store, folder);
  }
  await run(cmdUpdateAPIFolder, {
    folder,
  });
}

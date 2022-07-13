import dayjs from "dayjs";
import { ulid } from "ulid";

import { isWebMode } from "../helpers/util";
import { fakeAdd, fakeList, fakeUpdate } from "./fake";
import {
  run,
  cmdAddAPICollection,
  cmdListAPICollection,
  cmdUpdateAPICollection,
} from "./invoke";

const store = "apiCollections";

export interface APICollection {
  [key: string]: unknown;
  id: string;
  // 名称
  name: string;
  // 描述
  description: string;
  // 创建时间
  createdAt: string;
  // 更新时间
  updatedAt: string;
}

export function newDefaultAPICollection(): APICollection {
  const id = ulid();
  return {
    id,
    name: "",
    description: "",
    createdAt: dayjs().format(),
    updatedAt: dayjs().format(),
  };
}

export async function createAPICollection(
  collection: APICollection
): Promise<void> {
  if (isWebMode()) {
    await fakeAdd<APICollection>(store, collection);
    return;
  }
  await run(cmdAddAPICollection, {
    collection,
  });
}

export async function listAPICollection(): Promise<APICollection[]> {
  if (isWebMode()) {
    return await fakeList<APICollection>(store);
  }
  return await run<APICollection[]>(cmdListAPICollection);
}

export async function updateAPICollection(collection: APICollection) {
  if (isWebMode()) {
    await fakeUpdate(store, collection);
    return;
  }
  await run(cmdUpdateAPICollection, {
    collection,
  });
}

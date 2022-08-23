import { readTextFile } from "@tauri-apps/api/fs";
import { Promise } from "bluebird";
import { open } from "@tauri-apps/api/dialog";

import { SettingType } from "../stores/api_setting";
import { APIFolder, createAPIFolder, newDefaultAPIFolder } from "./api_folder";
import {
  APISetting,
  createAPISetting,
  newDefaultAPISetting,
} from "./api_setting";
import { ContentType, HTTPRequest } from "./http_request";
import { KVParam } from "./interface";

interface PostManSetting {
  name: string;
  item?: PostManSetting[];
  request?: {
    method: string;
    url: {
      raw: string;
    };
    query: {
      key: string;
      value: string;
    }[];
    body: {
      mode: string;
      raw: string;
    };
  };
}

interface ImportData {
  settings: APISetting[];
  folders: APIFolder[];
}

export enum ImportCategory {
  PostMan = "postMan",
}

function convertPostManAPISetting(item: PostManSetting, collection: string) {
  if (!item.request) {
    return;
  }
  const setting = newDefaultAPISetting();
  setting.category = SettingType.HTTP;
  setting.collection = collection;
  setting.name = item.name;
  let contentType = "";
  const body = item.request.body?.raw;
  if (body && body.startsWith("{") && body.endsWith("}")) {
    contentType = ContentType.JSON;
  }
  const query: KVParam[] = [];
  item.request.query?.forEach((q) => {
    query.push({
      key: q.key,
      value: q.value,
      enabled: true,
    });
  });
  let uri = item.request.url?.raw || "";
  if (uri && uri.includes("?")) {
    const arr = uri.split("?");
    uri = arr[0];
    // 去除前面host+path部分
    arr.shift();
    const url = new URL(`http://localhost/?${arr.join("?")}`);
    url.searchParams.forEach((value, key) => {
      query.push({
        key,
        value,
        enabled: true,
      });
    });
  }

  const req: HTTPRequest = {
    headers: [],
    method: item.request.method,
    uri,
    contentType,
    query,
    body: body,
  };
  setting.setting = JSON.stringify(req);
  return setting;
}

function convertPostManSetting(params: {
  result: ImportData;
  items: PostManSetting[];
  collection: string;
  parentChildren: string[];
}) {
  const { result, items, collection, parentChildren } = params;
  if (!items || items.length === 0) {
    return;
  }
  items.forEach((item) => {
    // api 接口
    if (item.request) {
      const setting = convertPostManAPISetting(item, collection);
      if (setting) {
        result.settings.push(setting);
        parentChildren.push(setting.id);
      }
    } else {
      // folder
      const folder = newDefaultAPIFolder();
      result.folders.push(folder);
      parentChildren.push(folder.id);
      folder.collection = collection;
      folder.name = item.name;
      const subChildren: string[] = [];
      convertPostManSetting({
        result,
        items: item.item || [],
        collection,
        parentChildren: subChildren,
      });
      folder.children = subChildren.join(",");
    }
  });
}

export async function importAPI(params: {
  category: ImportCategory;
  collection: string;
}) {
  const selected = await open({
    filters: [
      {
        name: "JSON",
        extensions: ["json"],
      },
    ],
  });
  if (!selected) {
    return;
  }

  const { collection, category } = params;
  const fileData = await readTextFile(selected as string);
  const result: ImportData = {
    settings: [],
    folders: [],
  };
  const json = JSON.parse(fileData);

  switch (category) {
    case ImportCategory.PostMan:
      {
        if (!Array.isArray(json.item)) {
          return;
        }
        const arr = json.item as PostManSetting[];
        convertPostManSetting({
          result,
          items: arr,
          collection,
          parentChildren: [],
        });
      }
      break;
    default:
      throw new Error(`${category} is not supported`);
      break;
  }

  await Promise.each(result.folders, async (item) => {
    await createAPIFolder(item);
  });
  await Promise.each(result.settings, async (item) => {
    await createAPISetting(item);
  });
}

import { readTextFile } from "@tauri-apps/api/fs";
import { Promise } from "bluebird";
import { open } from "@tauri-apps/api/dialog";
import { get } from "lodash-es";
import { MessageApiInjection } from "naive-ui/es/message/src/MessageProvider";

import { SettingType } from "../stores/api_setting";
import { APIFolder, createAPIFolder, newDefaultAPIFolder } from "./api_folder";
import {
  APISetting,
  createAPISetting,
  newDefaultAPISetting,
} from "./api_setting";
import { ContentType, HTTPRequest } from "./http_request";
import { KVParam } from "./interface";
import { i18nCollection } from "../i18n";

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

interface InsomniaSetting {
  parentId: string;
  url: string;
  name: string;
  method: string;
  sort: number;
  body: {
    mimeType: string;
    text: string;
  };
  headers: {
    name: string;
    value: string;
  }[];
  parameters: {
    name: string;
    value: string;
  }[];
  _id: string;
  _type: string;
}

interface ImportData {
  settings: APISetting[];
  folders: APIFolder[];
}

export enum ImportCategory {
  PostMan = "postMan",
  Insomnia = "insomnia",
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
  // TODO headers的处理
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
    auth: [],
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

function convertInsomniaSetting(params: {
  result: ImportData;
  items: InsomniaSetting[];
  collection: string;
}) {
  const { result, collection, items } = params;
  const subChildrenMap: Map<string, string[]> = new Map();
  const parentIDMap: Map<string, string> = new Map();
  const addToParent = (id: string, parentID: string) => {
    const pid = parentIDMap.get(parentID);
    if (pid) {
      const arr = subChildrenMap.get(pid) || [];
      arr.push(id);
      subChildrenMap.set(pid, arr);
    }
  };
  items.forEach((item) => {
    if (item._type === "request_group") {
      // folder
      const folder = newDefaultAPIFolder();
      result.folders.push(folder);
      folder.collection = collection;
      folder.name = item.name;
      subChildrenMap.set(folder.id, []);
      parentIDMap.set(item._id, folder.id);
      addToParent(folder.id, item.parentId);
      return;
    }
    const setting = newDefaultAPISetting();
    setting.category = SettingType.HTTP;
    setting.collection = collection;
    setting.name = item.name;
    const body = item.body.text;
    let contentType = "";
    if (body && body.startsWith("{") && body.endsWith("}")) {
      contentType = ContentType.JSON;
    }
    const query: KVParam[] = [];
    item.parameters?.forEach((q) => {
      query.push({
        key: q.name,
        value: q.value,
        enabled: true,
      });
    });
    const headers: KVParam[] = [];
    item.headers?.forEach((q) => {
      headers.push({
        key: q.name,
        value: q.value,
        enabled: true,
      });
    });
    let uri = item.url || "";
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
      headers: headers,
      method: item.method,
      uri,
      contentType,
      query,
      body: body,
      auth: [],
    };
    setting.setting = JSON.stringify(req);
    addToParent(setting.id, item.parentId);
    result.settings.push(setting);
  });

  result.folders.forEach((folder) => {
    const arr = subChildrenMap.get(folder.id);
    if (arr) {
      folder.children = arr.join(",");
    }
  });
}

export async function importAPI(params: {
  category: ImportCategory;
  collection: string;
  message: MessageApiInjection;
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
    return false;
  }

  const { collection, category, message } = params;
  const d = message.loading(i18nCollection("importing"));
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
          return false;
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
    case ImportCategory.Insomnia:
      {
        const items = get(json, "resources");
        if (!Array.isArray(items)) {
          return false;
        }
        let arr = items as InsomniaSetting[];
        arr.forEach((item) => {
          if (item._type === "request_group") {
            if (item.parentId.startsWith("wrk_")) {
              item.sort = 0;
            } else {
              item.sort = 1;
            }
          } else {
            item.sort = 2;
          }
        });
        arr = arr.filter((item) =>
          ["request", "request_group"].includes(item._type)
        );

        arr.sort((item1, item2) => {
          return item1.sort - item2.sort;
        });
        convertInsomniaSetting({
          result,
          items: arr,
          collection,
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
  d.destroy();
  return true;
}

import { Promise } from "bluebird";
import { get, uniq, forEach } from "lodash-es";

import { SettingType } from "../stores/api_setting";
import { APIFolder, createAPIFolder, newDefaultAPIFolder } from "./api_folder";
import {
  APISetting,
  createAPISetting,
  newDefaultAPISetting,
} from "./api_setting";
import { ContentType, HTTPRequest } from "./http_request";
import { KVParam } from "./interface";
import {
  Environment,
  newDefaultEnvironment,
  createEnvironment,
} from "../commands/environment";

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
  data: Map<string, string>;
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
  File = "file",
  Text = "text",
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
      if (!q.value && !q.name) {
        return;
      }
      query.push({
        key: q.name,
        value: q.value,
        enabled: true,
      });
    });
    const headers: KVParam[] = [];
    item.headers?.forEach((q) => {
      if (!q.value && !q.name) {
        return;
      }
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
  fileData: string;
}): Promise<string[]> {
  const { collection, category } = params;
  const result: ImportData = {
    settings: [],
    folders: [],
  };
  const json = JSON.parse(params.fileData);
  const environments: Environment[] = [];

  switch (category) {
    case ImportCategory.PostMan:
      {
        if (!Array.isArray(json.item)) {
          return [];
        }
        const arr = json.item as PostManSetting[];
        convertPostManSetting({
          result,
          items: arr,
          collection,
          parentChildren: [],
        });

        forEach(json.variable as [], (item: { key: string; value: string }) => {
          const env = newDefaultEnvironment();
          env.name = item.key;
          env.value = item.value;
          environments.push(env);
        });
      }
      break;
    case ImportCategory.Insomnia:
      {
        const items = get(json, "resources");
        if (!Array.isArray(items)) {
          return [];
        }
        let arr = items as InsomniaSetting[];
        arr.forEach((item) => {
          if (item._type === "environment") {
            forEach(item.data, (value, key) => {
              const env = newDefaultEnvironment();
              env.name = key;
              env.value = value as string;
              environments.push(env);
            });
            return;
          }
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
    case ImportCategory.Text:
    case ImportCategory.File:
      {
        const arr = Array.isArray(json) ? json : [json];
        arr.forEach((item) => {
          item.collection = collection;
          if (item.category === SettingType.HTTP) {
            result.settings.push(item);
          } else {
            result.folders.push(item);
          }
        });
      }
      break;
    default:
      throw new Error(`${category} is not supported`);
      break;
  }

  const topIDList: string[] = [];
  const childrenIDMap: Map<string, boolean> = new Map();
  result.folders.forEach((item) => {
    const children = item.children?.split(",") || [];
    children.forEach((id) => {
      childrenIDMap.set(id, true);
    });
  });

  await Promise.each(result.folders, async (item) => {
    if (!childrenIDMap.has(item.id)) {
      topIDList.push(item.id);
    }
    await createAPIFolder(item);
  });
  await Promise.each(result.settings, async (item) => {
    if (!childrenIDMap.has(item.id)) {
      topIDList.push(item.id);
    }
    await createAPISetting(item);
  });
  await Promise.each(environments, async (item) => {
    if (!item.name && !item.value) {
      return;
    }
    await createEnvironment(item);
  });
  return uniq(topIDList);
}

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

// "name": "对账单订阅一期",
// 			"item": [
// 				{
// 					"name": "原始接口",
// 					"item": [
// 						{
// 							"name": "金管家",
// 							"item": [

interface PostManSetting {
  name: string;
  item: {
    name: string;
    item?: PostManSetting[];
    request: {
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
  }[];
}

interface ImportData {
  settings: APISetting[];
  folders: APIFolder[];
}

export enum ImportCategory {
  PostMan = "postMan",
}

function convertPostManSetting(params: {
  result: ImportData;
  items: PostManSetting[];
  collection: string;
  parentChild: string[];
}) {
  const { result, items, collection, parentChild } = params;
  if (!items || items.length === 0) {
    return;
  }
  items.forEach((item) => {
    const folder = newDefaultAPIFolder();
    parentChild.push(folder.id);
    result.folders.push(folder);

    folder.collection = collection;
    folder.name = item.name;
    const children: string[] = [];
    const apiItems = item.item || [];
    apiItems.forEach((apiItem) => {
      if (!apiItem.request) {
        if (apiItem.item) {
          convertPostManSetting({
            result,
            items: apiItem.item,
            collection,
            parentChild: children,
          });
        }
        return;
      }
      const setting = newDefaultAPISetting();
      result.settings.push(setting);

      children.push(setting.id);
      setting.category = SettingType.HTTP;
      setting.collection = collection;
      setting.name = apiItem.name;

      let contentType = "";
      const body = apiItem.request.body?.raw;
      if (body && body.startsWith("{") && body.endsWith("}")) {
        contentType = ContentType.JSON;
      }
      const query: KVParam[] = [];
      apiItem.request.query?.forEach((q) => {
        query.push({
          key: q.key,
          value: q.value,
          enabled: true,
        });
      });
      const req: HTTPRequest = {
        headers: [],
        method: apiItem.request.method,
        uri: apiItem.request.url?.raw || "",
        contentType,
        query,
        body: body,
      };
      setting.setting = JSON.stringify(req);
    });
    folder.children = children.join(",");
  });
  return result;
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
          parentChild: [],
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

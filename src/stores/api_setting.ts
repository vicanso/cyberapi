import { defineStore } from "pinia";
import dayjs from "dayjs";
import { compact, uniq } from "lodash-es";

import {
  APIFolder,
  createAPIFolder,
  listAPIFolder,
  newDefaultAPIFolder,
  updateAPIFolder,
} from "../commands/api_folder";

import {
  APISetting,
  listAPISetting,
  newDefaultAPISetting,
  createAPISetting,
  updateAPISetting,
} from "../commands/api_setting";
import { i18nAppSetting } from "../i18n";

export interface APISettingTree {
  label: string;
  key: string;
  category: string;
  children: APISettingTree[];
}

export enum SettingType {
  HTTP = "http",
  Folder = "folder",
}

const rootPathID = "_";

export const useAPISettingsStore = defineStore("apiSettings", {
  state: () => {
    return {
      apiSettingMap: new Map<string, APISetting>(),
      apiFolderMap: new Map<string, APIFolder>(),
      apiSettingTrees: [] as APISettingTree[],
      addFolderProcessing: false,
      addProcessing: false,
      listProcessing: false,
      updateProcessing: false,
    };
  },
  actions: {
    async newRootFolder(): Promise<APIFolder> {
      const folder = newDefaultAPIFolder();
      folder.id = rootPathID;
      await createAPIFolder(folder);
      this.apiFolderMap.set(folder.id, folder);
      return folder;
    },
    async addToFolder(id: string, folder: string) {
      const { apiFolderMap } = this;
      let item = apiFolderMap.get(folder);
      if (folder == rootPathID && !item) {
        item = await this.newRootFolder();
      }
      if (!item) {
        return;
      }
      // 如果原来此id已经添加至其它的folder，先清除
      apiFolderMap.forEach(async (item) => {
        // 清除此id
        if (item.children.includes(id)) {
          const children = compact(item.children.split(","));
          item.children = children.join(",");
          await updateAPIFolder(item);
        }
      });

      const children = compact(item.children.split(","));
      children.push(id);
      item.children = uniq(children).join(",");
      await updateAPIFolder(item);
    },
    async addFolder(name: string, parent = rootPathID) {
      if (this.addFolderProcessing) {
        return;
      }
      const folder = newDefaultAPIFolder();
      folder.name = name;
      this.addFolderProcessing = true;
      try {
        await createAPIFolder(folder);
        await this.addToFolder(folder.id, parent);
      } finally {
        this.addFolderProcessing = false;
      }
    },
    async add(folder: string) {
      if (this.addProcessing) {
        return;
      }
      if (!folder) {
        folder = rootPathID;
      }
      const setting = newDefaultAPISetting();
      this.addProcessing = true;
      setting.folder = folder;
      try {
        await createAPISetting(setting);
        await this.addToFolder(setting.id, folder);
      } finally {
        this.addProcessing = false;
      }
    },
    async list(): Promise<void> {
      if (this.listProcessing) {
        return;
      }
      this.listProcessing = true;
      const defaultLabelName = i18nAppSetting("defaultName");
      try {
        const apiSettings = await listAPISetting();
        const apiFolders = await listAPIFolder();
        const treeMap = new Map<string, APISettingTree>();
        const apiSettingMap = new Map<string, APISetting>();
        apiSettings.forEach((item) => {
          apiSettingMap.set(item.id, item);
          treeMap.set(item.id, {
            label: item.name || defaultLabelName,
            key: item.id,
            category: SettingType.HTTP,
            children: [],
          });
        });
        const apiFolderMap = new Map<string, APIFolder>();
        const folderMap = new Map<string, APIFolder>();
        apiFolders.forEach((item) => {
          apiFolderMap.set(item.id, item);
          if (item.id == rootPathID) {
            return;
          }
          folderMap.set(item.id, item);
          treeMap.set(item.id, {
            label: item.name || defaultLabelName,
            key: item.id,
            category: SettingType.Folder,
            children: [],
          });
        });
        const removeIDList: string[] = [];
        folderMap.forEach((item) => {
          const tree = treeMap.get(item.id);
          if (!tree) {
            return;
          }
          const children = item.children.split(",");
          children.forEach((id) => {
            removeIDList.push(id);
            const child = treeMap.get(id);
            if (!child) {
              return;
            }
            tree.children.push(child);
          });
        });
        const apiSettingTrees: APISettingTree[] = [];
        treeMap.forEach((item) => {
          if (removeIDList.includes(item.key)) {
            return;
          }
          apiSettingTrees.push(item);
        });
        this.apiSettingTrees = apiSettingTrees;
        this.apiSettingMap = apiSettingMap;
        this.apiFolderMap = apiFolderMap;
      } finally {
        this.listProcessing = false;
      }
    },
    async update(data: APISetting) {
      if (this.updateProcessing) {
        return;
      }
      this.updateProcessing = true;
      data.updatedAt = dayjs().format();
      try {
        await updateAPISetting(data);
      } finally {
        this.updateProcessing = false;
      }
    },
  },
});

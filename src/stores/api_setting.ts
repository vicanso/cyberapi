import { defineStore } from "pinia";
import {
  APIFolder,
  createAPIFolder,
  listAPIFolder,
  newDefaultAPIFolder,
} from "../commands/api_folder";

import {
  APISetting,
  listAPISetting,
  newDefaultAPISetting,
  createAPISetting,
  updateAPISetting,
} from "../commands/api_setting";

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

export const useAPISettingsStore = defineStore("apiSettings", {
  state: () => {
    return {
      apiSettingMap: new Map<string, APISetting>(),
      apiSettingTrees: [] as APISettingTree[],
      addFolderProcessing: false,
      addProcessing: false,
      listProcessing: false,
      updateProcessing: false,
    };
  },
  actions: {
    async addFolder(name: string) {
      if (this.addFolderProcessing) {
        return;
      }
      const folder = newDefaultAPIFolder();
      folder.name = name;
      this.addFolderProcessing = true;
      try {
        await createAPIFolder(folder);
      } finally {
        this.addFolderProcessing = false;
      }
    },
    async add(folder = "") {
      if (this.addProcessing) {
        return;
      }
      const setting = newDefaultAPISetting();
      this.addProcessing = true;
      setting.folder = folder;
      try {
        await createAPISetting(setting);
      } finally {
        this.addProcessing = false;
      }
    },
    async list(): Promise<void> {
      if (this.listProcessing) {
        return;
      }
      this.listProcessing = true;
      try {
        const apiSettings = await listAPISetting();
        const apiFolders = await listAPIFolder();
        const treeMap = new Map<string, APISettingTree>();
        const parentMap = new Map<string, string>();
        apiFolders.forEach((item) => {
          treeMap.set(item.id, {
            key: item.id,
            label: item.name,
            category: SettingType.Folder,
            children: [],
          });
          if (item.parent) {
            parentMap.set(item.id, item.parent);
          }
        });
        const apiSettingMap = new Map<string, APISetting>();
        apiSettings.forEach((item) => {
          treeMap.set(item.id, {
            key: item.id,
            label: item.name,
            category: SettingType.HTTP,
            children: [],
          });
          apiSettingMap.set(item.id, item);
          if (item.folder) {
            parentMap.set(item.id, item.folder);
          }
        });
        const treeList: APISettingTree[] = [];
        treeMap.forEach((value, key) => {
          const tree = treeMap.get(key);
          if (tree) {
            const parentID = parentMap.get(key);
            // 如果有父元素，直接添加至children中
            if (parentID) {
              treeMap.get(parentID)?.children.push(tree);
              return;
            }
            treeList.push(tree);
          }
        });
        this.apiSettingTrees = treeList;
        this.apiSettingMap = apiSettingMap;
      } finally {
        this.listProcessing = false;
      }
    },
    async update(setting: APISetting) {
      if (this.updateProcessing) {
        return;
      }
      this.updateProcessing = true;
      try {
        await updateAPISetting(setting);
      } finally {
        this.updateProcessing = false;
      }
    },
  },
});

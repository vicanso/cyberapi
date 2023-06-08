import dayjs from "dayjs";
import { defineStore } from "pinia";
import { compact, uniq } from "lodash-es";
import {
  createAPIFolder,
  deleteAPIFolder,
  listAPIFolder,
  updateAPIFolder,
} from "../commands/api_folder";
import { APIFolder } from "../commands/api_folder";
import { useAPISettingStore } from "./api_setting";

export const useAPIFolderStore = defineStore("apiFolders", {
  state: () => {
    return {
      apiFolders: [] as APIFolder[],
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    findByID(id: string): APIFolder {
      const index = this.apiFolders.findIndex((item) => item.id === id);
      return this.apiFolders[index];
    },
    async updateByID(id: string, data: unknown) {
      const index = this.apiFolders.findIndex((item) => item.id === id);
      const item = Object.assign(this.apiFolders[index], data);
      await this.update(item);
    },
    async add(data: APIFolder): Promise<void> {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPIFolder(data);
        const arr = this.apiFolders.slice(0);
        arr.push(data);
        this.apiFolders = arr;
      } finally {
        this.adding = false;
      }
    },
    async fetch(collection: string): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        const arr = await listAPIFolder(collection);
        arr.forEach((item) => {
          if (!item.children) {
            return;
          }
          item.children = uniq(item.children.split(",")).join(",");
        });
        this.apiFolders = arr;
      } finally {
        this.fetching = false;
      }
    },
    async addChild(params: {
      // folder的id(为空则表示添加至顶层)
      id: string;
      // 要添加的元素
      children: string[];
      // 添加在哪个元素之前
      before?: string;
    }) {
      if (this.updating) {
        return;
      }
      this.updating = true;
      const { id, children } = params;
      const { apiFolders } = this;
      // 如果folder不存在
      const currentFolder = apiFolders.find((item) => item.id === id);
      const omitChild = (children: string, child: string) => {
        const arr = children.split(",").filter((item) => item !== child);

        return uniq(compact(arr)).join(",");
      };
      const addChild = (
        children: string,
        child: string,
        beforeItem: string
      ) => {
        const arr = compact(children.split(","));
        const index = arr.indexOf(beforeItem);
        if (index === -1) {
          arr.push(child);
        } else {
          arr.splice(index, 0, child);
        }
        return uniq(compact(arr)).join(",");
      };

      const updateData: Map<string, APIFolder> = new Map();

      for (let i = 0; i < children.length; i++) {
        const child = children[i];
        // 查找此child是否有已有对应folder
        apiFolders.forEach((folder) => {
          if (folder.children?.includes(child)) {
            folder.children = omitChild(folder.children, child);
            updateData.set(folder.id, folder);
          }
        });
        if (currentFolder) {
          currentFolder.children = addChild(
            currentFolder.children,
            child,
            params.before || ""
          );
          updateData.set(currentFolder.id, currentFolder);
        }
      }
      const list: APIFolder[] = [];
      updateData.forEach((value) => {
        list.push(value);
      });
      try {
        for (let i = 0; i < list.length; i++) {
          await updateAPIFolder(list[i]);
        }
      } finally {
        this.updating = false;
      }
    },
    async update(data: APIFolder): Promise<void> {
      if (this.updating) {
        return;
      }
      this.updating = true;
      data.updatedAt = dayjs().format();
      try {
        await updateAPIFolder(data);
        const arr = this.apiFolders.slice(0);
        const index = arr.findIndex((item) => item.id === data.id);
        if (index !== -1) {
          arr[index] = data;
        }
        this.apiFolders = arr;
      } finally {
        this.updating = false;
      }
    },
    async remove(id: string): Promise<void> {
      if (this.removing) {
        return;
      }
      this.removing = true;
      try {
        const settingStore = useAPISettingStore();
        const result = await deleteAPIFolder(id);

        const folderIds = result.folders || [];
        const settingIds = result.settings || [];
        this.apiFolders = this.apiFolders.filter(
          (item) => !folderIds.includes(item.id)
        );
        settingStore.apiSettings = settingStore.apiSettings.filter((item) => {
          return !settingIds.includes(item.id);
        });
      } finally {
        this.removing = false;
      }
    },
  },
});

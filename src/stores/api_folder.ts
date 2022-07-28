import dayjs from "dayjs";
import { defineStore } from "pinia";
import {
  createAPIFolder,
  deleteAPIFolder,
  listAPIFolder,
  updateAPIFolder,
} from "../commands/api_folder";
import { APIFolder } from "../commands/api_folder";

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
    async fetch(): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        this.apiFolders = await listAPIFolder();
      } finally {
        this.fetching = false;
      }
    },
    async addChild(params: {
      // folder的id
      id: string;
      // 要添加的元素
      child: string;
      // 要添加的位置
      index: number;
    }): Promise<void> {
      if (this.updating) {
        return;
      }
      const { id, child } = params;
      let prevParentIndex = -1;
      let currentParentIndex = -1;
      const arr = this.apiFolders.slice(0);
      // 查找以前及现在的folder
      arr.forEach((item, index) => {
        if (item.children?.includes(child)) {
          prevParentIndex = index;
        }
        if (item.id === id) {
          currentParentIndex = index;
        }
      });
      if (currentParentIndex === -1) {
        return;
      }
      this.updating = true;
      try {
        if (prevParentIndex !== -1) {
          const prevParent = arr[prevParentIndex];
          const children = prevParent.children
            .split(",")
            .filter((item) => item !== child);
          prevParent.children = children.join(",");
          // 先清除原有记录
          await updateAPIFolder(prevParent);
        }
        // 添加至新的folder
        const currentParent = arr[currentParentIndex];
        const children = currentParent.children?.split(",");
        if (params.index === -1) {
          children.push(child);
        } else {
          children.splice(params.index, 0, child);
        }
        currentParent.children = children.join(",");
        await updateAPIFolder(currentParent);
        this.apiFolders = arr;
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
        await deleteAPIFolder(id);
      } finally {
        this.removing = false;
      }
    },
  },
});

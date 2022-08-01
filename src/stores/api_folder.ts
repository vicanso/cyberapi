import dayjs from "dayjs";
import { defineStore } from "pinia";
import { uniq } from "lodash-es";
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
    findByID(id: string) {
      return this.apiFolders.find((item) => item.id === id);
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
    async fetch(): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        const arr = await listAPIFolder();
        arr.forEach((item) => {
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
      child: string;
      // 要添加的位置
      index: number;
    }): Promise<void> {
      if (this.updating) {
        return;
      }

      // 同目录拖动顺序
      // 不同目录拖动
      // 将一个目录拖至另外一个目录

      const { id, child } = params;
      // 如果同元素，则忽略
      if (id === child) {
        return;
      }
      let prevParentIndex = -1;
      let currentParentIndex = -1;
      const arr = this.apiFolders;
      // 查找以前及现在的folder
      arr.forEach((item, index) => {
        if (item.children?.includes(child)) {
          prevParentIndex = index;
        }
        if (item.id === id) {
          currentParentIndex = index;
        }
      });

      this.updating = true;
      try {
        const currentParent = arr[currentParentIndex];
        const children = currentParent?.children?.split(",");
        // 同一目录，只调整顺序
        if (currentParent && prevParentIndex === currentParentIndex) {
          const currentIndex = children.indexOf(child);
          children.splice(currentIndex, 1);
        } else if (prevParentIndex !== -1) {
          const prevParent = arr[prevParentIndex];
          const children = prevParent.children
            .split(",")
            .filter((item) => item !== child);
          prevParent.children = uniq(children).join(",");
          // 先清除原有记录
          await updateAPIFolder(prevParent);
        }

        // 如果无匹配的folder，则表示添加至collection最顶层
        // 无需更新current parent
        if (currentParentIndex !== -1) {
          // 添加至新的folder
          if (params.index === -1) {
            children.push(child);
          } else {
            children.splice(params.index, 0, child);
          }
          currentParent.children = uniq(children).join(",");
          await updateAPIFolder(currentParent);
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
        await deleteAPIFolder(id);
      } finally {
        this.removing = false;
      }
    },
  },
});

import dayjs from "dayjs";
import { uniq } from "lodash-es";
import { defineStore } from "pinia";

import {
  APICollection,
  createAPICollection,
  updateAPICollection,
  listAPICollection,
  deleteAPICollection,
} from "../commands/api_collection";
import {
  getExpandedSettingStore,
  getTopTreeItemStore,
  getTabActiveStore,
} from "./local";

const tabActiveKey = "activeTabs";

interface TabActiveData {
  [key: string]: string;
}

async function toggleFolderExpanded(
  collection: string,
  folder: string,
  expanded: boolean,
) {
  let items = await getExpandedSettingStore().getItem<string[]>(collection);
  if (!items) {
    items = [];
  }
  if (expanded) {
    items.push(folder);
  } else {
    items = items.filter((item) => item !== folder);
  }
  items = uniq(items);
  await getExpandedSettingStore().setItem(collection, items);
  return items;
}

export const useAPICollectionStore = defineStore("apiCollections", {
  state: () => {
    return {
      apiCollections: [] as APICollection[],
      expandedFolders: [] as string[],
      topTreeItems: [] as string[],
      activeTabs: {} as TabActiveData,
      fetching: false,
      adding: false,
      updating: false,
      removing: false,
    };
  },
  actions: {
    findByID(id: string) {
      return this.apiCollections.find((item) => item.id === id);
    },
    async closeAllFolders(collection: string) {
      const items: string[] = [];
      await getExpandedSettingStore().setItem(collection, items);
      this.expandedFolders = items;
    },
    async fetchExpandedFolders(collection: string) {
      const items =
        await getExpandedSettingStore().getItem<string[]>(collection);
      if (items) {
        this.expandedFolders = items;
      }
    },
    async fetchTopTreeItems(collection: string) {
      const items = await getTopTreeItemStore().getItem<string[]>(collection);
      if (items) {
        this.topTreeItems = items;
      }
    },
    async fetchActiveTabs() {
      const data =
        await getTabActiveStore().getItem<TabActiveData>(tabActiveKey);
      this.activeTabs = data || {};
    },
    getActiveTab(id: string) {
      return this.activeTabs[id];
    },
    async updateActiveTab(params: { id: string; activeTab: string }) {
      const { id, activeTab } = params;
      // 已经相同
      if (this.activeTabs[id] === activeTab) {
        return;
      }
      if (activeTab) {
        this.activeTabs[id] = activeTab;
      } else {
        delete this.activeTabs[id];
      }
      await getTabActiveStore().setItem(
        tabActiveKey,
        Object.assign({}, this.activeTabs),
      );
    },
    async updateTopTreeItems(collection: string, idList: string[]) {
      await getTopTreeItemStore().setItem(collection, idList);
      this.topTreeItems = idList;
    },
    async openFolder(collection: string, folder: string) {
      // localforage操作较快，因此不记录处理中
      const items = await toggleFolderExpanded(collection, folder, true);
      this.expandedFolders = items;
    },
    async closeFolder(collection: string, folder: string) {
      // localforage操作较快，因此不记录处理中
      const items = await toggleFolderExpanded(collection, folder, false);
      this.expandedFolders = items;
    },
    async add(data: APICollection): Promise<void> {
      if (this.adding) {
        return;
      }
      this.adding = true;
      try {
        await createAPICollection(data);
        const arr = this.apiCollections.slice(0);
        arr.push(data);
        this.apiCollections = arr;
      } finally {
        this.adding = false;
      }
    },
    async get(id: string): Promise<APICollection | undefined> {
      if (!this.apiCollections.length) {
        await this.fetch();
      }
      return this.apiCollections.find((item) => item.id === id);
    },
    async fetch(): Promise<void> {
      if (this.fetching) {
        return;
      }
      this.fetching = true;
      try {
        this.apiCollections = await listAPICollection();
      } finally {
        this.fetching = false;
      }
    },
    async update(data: APICollection): Promise<void> {
      if (this.updating) {
        return;
      }
      this.updating = true;
      data.updatedAt = dayjs().format();
      try {
        await updateAPICollection(data);
        const arr = this.apiCollections.slice(0);
        const index = arr.findIndex((item) => item.id === data.id);
        if (index !== -1) {
          arr[index] = data;
        }
        this.apiCollections = arr;
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
        await deleteAPICollection(id);
        const arr = this.apiCollections.slice(0);
        const index = arr.findIndex((item) => item.id === id);
        if (index !== -1) {
          arr.splice(index, 1);
        }
        this.apiCollections = arr;
      } finally {
        this.removing = false;
      }
    },
  },
});

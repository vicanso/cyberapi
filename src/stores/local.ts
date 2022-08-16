import localforage from "localforage";

function createNewStore(name: string) {
  let store: LocalForage;
  return function () {
    if (!store) {
      store = localforage.createInstance({
        name,
      });
    }
    return store;
  };
}

// 记录展开配置项
export const getExpandedSettingStore = createNewStore("expandedSettingStore");

// 记录顶层的配置项
export const getTopTreeItemStore = createNewStore("topTreeItemStore");

// 记录活动的Tab
export const getTabActiveStore = createNewStore("tabActiveStore");

// API Setting的额外记录，如选中记录等
export const getAPISettingsStore = createNewStore("apiSettings");

// 应用配置项
export const getSettingStore = createNewStore("setting");

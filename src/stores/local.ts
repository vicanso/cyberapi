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

enum StoreKey {
  expandedSetting = "expandedSetting",
  topTreeItem = "topTreeItem",
  tabActive = "tabActive",
  apiSetting = "apiSetting",
  setting = "setting",
  pinRequests = "pinRequests",
  latestResponse = "latestResponse",
}

// 记录展开配置项
export const getExpandedSettingStore = createNewStore(StoreKey.expandedSetting);

// 记录顶层的配置项
export const getTopTreeItemStore = createNewStore(StoreKey.topTreeItem);

// 记录活动的Tab
export const getTabActiveStore = createNewStore(StoreKey.tabActive);

// API Setting的额外记录，如选中记录等
export const getAPISettingStore = createNewStore(StoreKey.apiSetting);

// 应用配置项
export const getSettingStore = createNewStore(StoreKey.setting);

// Pin的API配置
export const getPinRequestStore = createNewStore(StoreKey.pinRequests);

// 最新请求响应
export const getLatestResponseStore = createNewStore(StoreKey.latestResponse);

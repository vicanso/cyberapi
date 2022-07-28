import { defineComponent, ref, onBeforeMount } from "vue";
import { storeToRefs } from "pinia";

import { useAPIFolderStore } from "../../stores/api_folder";
import { showError } from "../../helpers/util";
import { useMessage } from "naive-ui";
import { useAPISettingStore, SettingType } from "../../stores/api_setting";
import { APIFolder } from "../../commands/api_folder";
import { APISetting } from "../../commands/api_setting";

interface TreeItem {
  id: string;
  name: string;
  settingType: string;
  children: TreeItem[];
}

function convertToTreeItems(
  apiFolders: APIFolder[],
  apiSettings: APISetting[]
): TreeItem[] {
  const map = new Map<string, TreeItem>();
  apiSettings.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: "",
      children: [],
    });
  });
  apiFolders.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: SettingType.Folder,
      children: [],
    });
  });
  // 记录已经设置为子元素的id
  const children = [] as string[];
  apiFolders.forEach((item) => {
    if (!item.children) {
      return;
    }
    const arr = item.children.split(",");
    const treeItem = map.get(item.id);
    if (!treeItem) {
      return;
    }
    arr.forEach((id) => {
      const subItem = map.get(id);
      if (!subItem) {
        return;
      }
      treeItem.children.push(subItem);
      children.push(id);
    });
  });
  const result = [] as TreeItem[];
  map.forEach((item, key) => {
    if (children.includes(key)) {
      return;
    }
    result.push(item);
  });
  return result;
}

export default defineComponent({
  name: "APISettingTreeItems",
  setup() {
    const message = useMessage();
    const processing = ref(false);
    const folderStore = useAPIFolderStore();
    const settingStore = useAPISettingStore();
    const { apiFolders } = storeToRefs(folderStore);
    const { apiSettings } = storeToRefs(settingStore);
    onBeforeMount(async () => {
      processing.value = true;
      try {
        await folderStore.fetch();
        await settingStore.fetch();
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    });

    return {
      apiFolders,
      apiSettings,
      processing,
    };
  },
  render() {
    const { apiFolders, apiSettings } = this;
    const treeItems = convertToTreeItems(apiFolders, apiSettings);
    const toList = (items: TreeItem[], index: number) => {
      if (!items || items.length === 0) {
        return;
      }
      const arr = items.map((item) => {
        const children = toList(item.children, index + 1);
        return (
          <li key={item.id}>
            {item.name}
            {children}
          </li>
        );
      });
      return <ul>{arr}</ul>;
    };
    return <div>{toList(treeItems, 0)}</div>;
  },
});

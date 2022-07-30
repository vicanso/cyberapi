import { defineComponent, ref, onBeforeMount } from "vue";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";
import { NIcon, useMessage } from "naive-ui";

import { useAPIFolderStore } from "../../stores/api_folder";
import { showError } from "../../helpers/util";
import { useAPISettingStore, SettingType } from "../../stores/api_setting";
import { APIFolder } from "../../commands/api_folder";
import { APISetting } from "../../commands/api_setting";
import {
  AnalyticsOutline,
  FolderOpenOutline,
  FolderOutline,
} from "@vicons/ionicons5";
import { useSettingStore } from "../../stores/setting";
import { useAPICollectionStore } from "../../stores/api_collection";
import { useRoute } from "vue-router";

const itemsClass = css`
  margin: 5px 0;
  padding: 0;
  list-style: none;
  li {
    padding: 8px 10px;
    line-height: 25px;
    height: 25px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    &:hover,
    &.selected {
      cursor: pointer;
      background-color: rgba(255, 255, 255, 0.1);
    }
    &.light:hover,
    &.selected.light {
      background-color: rgba(0, 0, 0, 0.1);
    }
  }
  .n-icon {
    float: left;
    font-size: 16px;
    line-height: 29px;
    margin: 0 5px 0 10px;
    font-weight: 900;
  }
`;
interface TreeItem {
  id: string;
  name: string;
  settingType: string;
  children: TreeItem[];
  expanded: boolean;
}

function convertToTreeItems(
  apiFolders: APIFolder[],
  apiSettings: APISetting[],
  expandedFolders: string[]
): TreeItem[] {
  const map = new Map<string, TreeItem>();
  apiSettings.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: "",
      children: [],
      expanded: false,
    });
  });
  apiFolders.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: SettingType.Folder,
      children: [],
      expanded: expandedFolders.includes(item.id),
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
  props: {
    keyword: {
      type: String,
      default: () => "",
    },
  },
  setup() {
    const message = useMessage();
    const processing = ref(false);
    const route = useRoute();
    const collection = route.query.id as string;

    const collectionStore = useAPICollectionStore();
    const folderStore = useAPIFolderStore();
    const settingStore = useAPISettingStore();
    const { apiFolders } = storeToRefs(folderStore);
    const { expandedFolders } = storeToRefs(collectionStore);
    const { isDark } = storeToRefs(useSettingStore());
    const { apiSettings } = storeToRefs(settingStore);

    const handleClick = async (item: TreeItem) => {
      try {
        // folder的处理
        if (item.settingType === SettingType.Folder) {
          let fn = collectionStore.openFolder;
          if (item.expanded) {
            fn = collectionStore.closeFolder;
          }
          await fn(collection, item.id);
        }
      } catch (err) {
        showError(message, err);
      }
    };

    onBeforeMount(async () => {
      processing.value = true;
      try {
        await collectionStore.fetchExpandedFolders(collection);
        await folderStore.fetch();
        await settingStore.fetch();
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    });

    return {
      expandedFolders,
      isDark,
      apiFolders,
      apiSettings,
      processing,
      handleClick,
    };
  },
  render() {
    const { keyword } = this.$props;
    const { apiFolders, apiSettings, isDark, expandedFolders } = this;
    const treeItems = convertToTreeItems(
      apiFolders,
      apiSettings,
      expandedFolders
    );
    const itemList = [] as JSX.Element[];
    const appendToList = (items: TreeItem[], index: number) => {
      if (!items || items.length === 0) {
        return;
      }
      items.forEach((item) => {
        // 暂时只过滤接口
        if (
          keyword &&
          item.settingType !== SettingType.Folder &&
          !item.name.toLocaleLowerCase().includes(keyword)
        ) {
          return;
        }
        let icon = (
          <NIcon>
            <AnalyticsOutline />
          </NIcon>
        );
        if (item.settingType === SettingType.Folder) {
          icon = (
            <NIcon>
              {item.expanded ? <FolderOpenOutline /> : <FolderOutline />}
            </NIcon>
          );
        }

        const style = {
          "padding-left": `${index * 20}px`,
        };
        const cls = isDark ? "" : "light";
        itemList.push(
          <li
            key={item.id}
            class={cls}
            style={style}
            onClick={() => {
              this.handleClick(item);
            }}
          >
            {icon}
            {item.name}
          </li>
        );
        // 未展开的则不需要展示子元素
        if (!item.expanded) {
          return;
        }
        appendToList(item.children, index + 1);
      });
    };
    appendToList(treeItems, 0);
    return <ul class={itemsClass}>{itemList}</ul>;
  },
});

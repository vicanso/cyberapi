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
import {
  addNodeClass,
  cloneNode,
  getNodeDataValue,
  getNodeOffset,
  getNodeOffsetHeightWidth,
  getNodeScrollTop,
  insertNodeAt,
  removeNode,
  removeNodeClass,
  setNodeStyle,
} from "../../helpers/html";

const itemsWrapperClass = css`
  position: absolute;
  top: 50px;
  left: 5px;
  right: 5px;
  bottom: 5px;
  overflow-y: auto;
  &.dragging {
    li:hover {
      background-color: rgba(255, 255, 255, 0) !important;
    }
    .dragItem {
      &:hover {
        background-color: rgba(255, 255, 255, 0.3) !important;
      }
      &.light:hover {
        background-color: rgba(0, 0, 0, 0.3) !important;
      }
    }
  }

  ul {
    margin: 0;
    padding: 0;
  }
  li {
    list-style: none;
    padding: 8px 10px;
    line-height: 25px;
    height: 25px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    &.insertBefore {
      padding-top: 7px;
      border-top: 1px dashed;
    }
    &:hover {
      cursor: pointer;
      background-color: rgba(255, 255, 255, 0.1);
    }
    &.light:hover {
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
    const wrapper = ref(null);
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

    const handleMove = (moveID: string, targetID: string) => {
      if (moveID === targetID) {
        return;
      }
      console.dir(moveID);
      console.dir(targetID);
    };

    let target: EventTarget;
    let moveTarget: EventTarget;
    let originClientY = 0;
    let originOffset = 0;
    let targetHeight = 0;
    let currentInsertIndex = -1;
    let isDragging = false;
    const draggingClass = "dragging";
    let listItems = [] as HTMLCollection[];
    const handleMousemove = (e: MouseEvent) => {
      // 每移动两个点再处理
      if (isDragging && e.clientY % 2 !== 0) {
        e.preventDefault();
        return;
      }
      const offset = e.clientY - originClientY;
      if (!isDragging && Math.abs(offset) > 3) {
        isDragging = true;
        addNodeClass(wrapper.value, draggingClass);
        moveTarget = cloneNode(target);
        setNodeStyle(moveTarget, {
          position: "absolute",
          width: "100%",
        });
        addNodeClass(moveTarget, "dragItem");

        insertNodeAt(wrapper.value, moveTarget, 0);
      }
      if (isDragging) {
        const top = offset + originOffset + getNodeScrollTop(wrapper.value);
        const index = Math.round(top / targetHeight);
        if (currentInsertIndex !== index) {
          if (currentInsertIndex !== -1) {
            removeNodeClass(listItems[currentInsertIndex], "insertBefore");
          }
          if (listItems.length > index) {
            addNodeClass(listItems[index], "insertBefore");
            currentInsertIndex = index;
          }
        }
        setNodeStyle(moveTarget, {
          top: `${top}px`,
        });
        e.preventDefault();
      }
    };

    const handleMouseup = () => {
      document.removeEventListener("mousemove", handleMousemove);
      document.removeEventListener("mouseup", handleMouseup);
      isDragging = false;
      const moveID = getNodeDataValue(moveTarget, "id");
      const targetID = getNodeDataValue(listItems[currentInsertIndex], "id");
      removeNode(moveTarget);
      removeNodeClass(listItems[currentInsertIndex], "insertBefore");
      removeNodeClass(wrapper.value, draggingClass);

      handleMove(moveID, targetID);
    };
    const handleMousedown = (e: MouseEvent) => {
      if (!e.currentTarget) {
        return;
      }
      // TODO 此处导致无法复制，后续研究
      e.preventDefault();
      currentInsertIndex = -1;
      target = e.currentTarget;
      originOffset =
        getNodeOffset(target).top - getNodeOffset(wrapper.value).top;
      targetHeight = getNodeOffsetHeightWidth(target).height;
      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      listItems = wrapper.value.children[0].children;
      originClientY = e.clientY;
      document.addEventListener("mousemove", handleMousemove);
      document.addEventListener("mouseup", handleMouseup);
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
      handleMousedown,
      wrapper,
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
            data-id={item.id}
            data-category={item.settingType}
            class={cls}
            style={style}
            onClick={() => {
              this.handleClick(item);
            }}
            onMousedown={this.handleMousedown}
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
    return (
      <div class={itemsWrapperClass} ref="wrapper">
        <ul>{itemList}</ul>
      </div>
    );
  },
});

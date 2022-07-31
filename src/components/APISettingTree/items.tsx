// API列表，实现拖动
import { defineComponent, ref, onBeforeMount } from "vue";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";
import { NIcon, useMessage } from "naive-ui";
import { sortBy, uniq } from "lodash-es";
import {
  AnalyticsOutline,
  FolderOpenOutline,
  FolderOutline,
} from "@vicons/ionicons5";
import { useRoute } from "vue-router";

import { useAPIFolderStore } from "../../stores/api_folder";
import { showError } from "../../helpers/util";
import { useAPISettingStore, SettingType } from "../../stores/api_setting";
import { APIFolder } from "../../commands/api_folder";
import { APISetting } from "../../commands/api_setting";

import { useSettingStore } from "../../stores/setting";
import { useAPICollectionStore } from "../../stores/api_collection";
import {
  nodeAddClass,
  nodeClone,
  nodeGetDataValue,
  nodeGetOffset,
  nodeGetOffsetHeightWidth,
  nodGetScrollTop,
  nodeInsertAt,
  nodeRemove,
  nodeRemoveClass,
  nodeSetStyle,
  nodeHasClass,
  nodeGetTagName,
} from "../../helpers/html";
import ExLoading from "../ExLoading";
import APISettingTreeItemDropdown from "./item_dropdown";

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
      .itemDropitem {
        display: inline;
      }
    }
    &.light:hover {
      background-color: rgba(0, 0, 0, 0.1);
    }
    .itemDropitem {
      float: right;
      display: none;
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

enum OverType {
  Over = 0,
  Top = 1,
  Bottom = 2,
}

interface TreeItem {
  id: string;
  name: string;
  settingType: string;
  children: TreeItem[];
  expanded: boolean;
  parent: string;
  childIndex: number;
}

function convertToTreeItems(
  apiFolders: APIFolder[],
  apiSettings: APISetting[],
  expandedFolders: string[],
  topTreeItems: string[]
): TreeItem[] {
  const map = new Map<string, TreeItem>();
  apiSettings.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: "",
      children: [],
      expanded: false,
      parent: "",
      childIndex: -1,
    });
  });
  apiFolders.forEach((item) => {
    map.set(item.id, {
      id: item.id,
      name: item.name,
      settingType: SettingType.Folder,
      children: [],
      expanded: expandedFolders.includes(item.id),
      parent: "",
      childIndex: -1,
    });
  });
  // 记录已经设置为子元素的id
  const children = [] as string[];
  apiFolders.forEach((item) => {
    if (!item.children) {
      return;
    }
    const treeItem = map.get(item.id);
    if (!treeItem) {
      return;
    }
    item.children.split(",").forEach((child) => {
      if (child === treeItem.id) {
        return;
      }

      const subItem = map.get(child);
      if (!subItem) {
        return;
      }
      subItem.parent = treeItem.id;
      subItem.childIndex = treeItem.children.length;
      treeItem.children.push(subItem);
      children.push(child);
    });
  });
  const result = [] as TreeItem[];
  map.forEach((item, key) => {
    if (children.includes(key)) {
      return;
    }
    result.push(item);
  });
  return sortBy(result, (item) => {
    return topTreeItems.indexOf(item.id);
  });
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
    const { expandedFolders, topTreeItems } = storeToRefs(collectionStore);
    const { isDark } = storeToRefs(useSettingStore());
    const { apiSettings } = storeToRefs(settingStore);

    let currentTreeItems = [] as TreeItem[];
    let topTreeItemIDList = [] as string[];

    const setTreeItems = (items: TreeItem[], topItems: string[]) => {
      currentTreeItems = items;
      topTreeItemIDList = topItems;
    };

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

    const handleMove = async (
      moveIndex: number,
      targetIndex: number,
      overType: OverType
    ) => {
      // TODO 如果是最后一个元素的处理
      // isOver move 与 target 是否重叠
      const moveItem = currentTreeItems[moveIndex];
      const targetItem = currentTreeItems[targetIndex];
      // 如果元素不存在，则忽略不处理
      if (!moveItem || !targetItem) {
        return;
      }
      // 同一个元素不处理
      if (moveItem.id === targetItem.id) {
        return;
      }
      let parentID = targetItem.parent;
      let childIndex = targetItem.childIndex;
      // 如果target是目录，而且是over
      // 则表示拖至此目录
      if (
        targetItem.settingType === SettingType.Folder &&
        overType === OverType.Over
      ) {
        parentID = targetItem.id;
        childIndex = 0;
      }
      try {
        // 记录顶层顺序
        if (!parentID) {
          let targetIndex = topTreeItemIDList.indexOf(targetItem.id);
          if (targetIndex < 0) {
            targetIndex = 0;
          }
          const moveIndex = topTreeItemIDList.indexOf(moveItem.id);
          //  如果是后移
          if (moveIndex !== -1 && moveIndex < targetIndex) {
            topTreeItemIDList.splice(moveIndex, 1);
            targetIndex--;
          }

          topTreeItemIDList.splice(targetIndex, 0, moveItem.id);
          await collectionStore.updateTopTreeItems(
            collection,
            uniq(topTreeItemIDList)
          );
        }
        await folderStore.addChild({
          id: parentID,
          child: moveItem.id,
          index: childIndex,
        });
      } catch (err) {
        showError(message, err);
      }
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
        nodeAddClass(wrapper.value, draggingClass);
        moveTarget = nodeClone(target);
        nodeSetStyle(moveTarget, {
          position: "absolute",
          width: "100%",
        });
        nodeAddClass(moveTarget, "dragItem");

        nodeInsertAt(wrapper.value, moveTarget, 0);
      }
      if (isDragging) {
        const top = offset + originOffset + nodGetScrollTop(wrapper.value);
        const index = Math.round(top / targetHeight);
        if (currentInsertIndex !== index) {
          if (currentInsertIndex !== -1) {
            nodeRemoveClass(listItems[currentInsertIndex], "insertBefore");
          }
          if (listItems.length > index) {
            nodeAddClass(listItems[index], "insertBefore");
            currentInsertIndex = index;
          }
        }
        nodeSetStyle(moveTarget, {
          top: `${top}px`,
        });
        e.preventDefault();
      }
    };

    const handleMouseup = (e: MouseEvent) => {
      document.removeEventListener("mousemove", handleMousemove);
      document.removeEventListener("mouseup", handleMouseup);
      if (!isDragging) {
        return;
      }

      let overType = OverType.Bottom;
      const overOffset = 5;
      const offset = Math.abs(e.clientY - originClientY) % targetHeight;
      // 覆盖
      if (offset <= overOffset || targetHeight - offset <= overOffset) {
        overType = OverType.Over;
      } else if (offset < targetHeight * 0.4) {
        overType = OverType.Top;
      }

      isDragging = false;

      const moveItemIndex = Number.parseInt(
        nodeGetDataValue(moveTarget, "index")
      );
      const targetItemIndex = Number.parseInt(
        nodeGetDataValue(listItems[currentInsertIndex], "index")
      );

      nodeRemove(moveTarget);
      nodeRemoveClass(listItems[currentInsertIndex], "insertBefore");
      nodeRemoveClass(wrapper.value, draggingClass);

      // 最后一个元素，而且移动至最后，则暂不处理
      // TODO 后续优化
      if (
        targetItemIndex === currentTreeItems.length - 1 &&
        overType === OverType.Bottom
      ) {
        return;
      }

      handleMove(moveItemIndex, targetItemIndex, overType);
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
        nodeGetOffset(target).top - nodeGetOffset(wrapper.value).top;
      targetHeight = nodeGetOffsetHeightWidth(target).height;
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
        await collectionStore.fetchTopTreeItems(collection);
        await folderStore.fetch();
        await settingStore.fetch();
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    });

    return {
      topTreeItems,
      expandedFolders,
      isDark,
      apiFolders,
      apiSettings,
      processing,
      handleClick,
      handleMousedown,
      setTreeItems,
      wrapper,
    };
  },
  render() {
    const { keyword } = this.$props;
    const {
      apiFolders,
      apiSettings,
      isDark,
      expandedFolders,
      processing,
      topTreeItems,
      setTreeItems,
    } = this;
    if (processing) {
      return <ExLoading />;
    }
    const treeItems = convertToTreeItems(
      apiFolders,
      apiSettings,
      expandedFolders,
      topTreeItems
    );
    const itemList = [] as JSX.Element[];
    // 当前展示的tree item
    const currentTreeItems = [] as TreeItem[];
    // 顶层元素
    const topTreeItemIDList = [] as string[];
    let treeItemIndex = 0;
    const appendToList = (items: TreeItem[], level: number) => {
      if (!items || items.length === 0) {
        return;
      }
      items.forEach((item) => {
        if (level === 0) {
          topTreeItemIDList.push(item.id);
        }
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
          "padding-left": `${level * 20}px`,
        };
        const cls = isDark ? "" : "light";
        currentTreeItems.push(item);
        itemList.push(
          <li
            key={`${item.id}-${level}`}
            data-index={treeItemIndex}
            class={cls}
            style={style}
            onClick={(e) => {
              const { target } = e;
              if (
                nodeHasClass(target, "preventDefault") ||
                nodeGetTagName(target) === "path"
              ) {
                e.preventDefault();
                return;
              }
              this.handleClick(item);
            }}
            onMousedown={this.handleMousedown}
          >
            {icon}
            {item.name}
            <APISettingTreeItemDropdown
              id={item.id}
              apiSettingType={item.settingType}
            />
          </li>
        );
        treeItemIndex++;
        // 未展开的则不需要展示子元素
        if (!item.expanded) {
          return;
        }
        appendToList(item.children, level + 1);
      });
    };
    appendToList(treeItems, 0);
    setTreeItems(currentTreeItems, topTreeItemIDList);
    return (
      <div class={itemsWrapperClass} ref="wrapper">
        <ul>{itemList}</ul>
      </div>
    );
  },
});

// API列表，实现拖动
import { defineComponent, ref, onBeforeUnmount } from "vue";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";
import { NGradientText, NInput, useMessage } from "naive-ui";
import { sortBy, uniq } from "lodash-es";
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
import APISettingTreeItemDropdown from "./item_dropdown";
import { HTTPMethod } from "../../commands/http_request";

const itemsWrapperClass = css`
  user-select: none;
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
    padding: 3px 10px;
    line-height: 34px;
    height: 34px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;

    .method {
      margin: 0 8px 0 5px;
      font-size: 12px;
    }
    &.renameItem {
      // 避免出现...
      text-overflow: initial;
      &:hover .itemDropitem {
        display: none;
      }
    }
    .renameInput {
      width: 86% !important;
    }
    &.insertBefore {
      padding-top: 2px;
      border-top: 1px dashed;
    }
    &:hover {
      cursor: pointer;
      background-color: rgba(255, 255, 255, 0.1);
      .itemDropitem {
        display: inline;
      }
    }
    &.selected {
      background-color: rgba(255, 255, 255, 0.1);
      color: #fff;
    }
    &.light:hover {
      background-color: rgba(30, 30, 30, 0.1);
    }
    &.light.selected {
      background-color: rgba(30, 30, 30, 0.1);
      color: #000;
    }
    .itemDropitem {
      float: right;
      display: none;
    }
    .folder {
      display: block;
      float: left;
      width: 30px;
      height: 100%;
      background-size: 18px;
      background-repeat: no-repeat;
      background-position: center;
      &.close {
        background-image: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAAABmJLR0QA/wD/AP+gvaeTAAABf0lEQVRoge2ZPUvDUBSG39h84CyU1ioOMXToILWCf8Ctf8DBTfE3OBoUwUHEwUEHQcRf4u7iIogIOtihflBQ8tHkOhVyAzVgTtMmngcynCS89z73ngRCAIZhGCYFSrR4vUJbAc4EUPtjngDwAAUH1Q1cpp9eMpLA8wVeAMxRBAsBe2ETuxRZvyEJPJ5DEOcfmtvYIc6UkATuT8kFIIDj0MB+Ywvv1NlATODuhF6gbLZQNlcARUm+OYG+8x30uk/2zNrN3uCclHp7RCtQtVqoWC3KSPjOV6A3r9VBrUoXfbqBavVlVBabgAjpQgFoxnQpWssC/fQDqLqB+foSZq0GgCB9YNJ40WK1vQ5VN2iSiVd+GJKAqmmZDUyFKpf5mjwQF8jZ6gO8A+OnYALcQtlTMIG8t5DgHciegj0DuRfgFsqemMDov6Co4RYaNwUT4BbKnmLtgOe4gW5opWE3TwKe40vv+qlo8dHp2Z7jBhAhJvHwHDfovn2O/J8DwzDMP+IHy6i/03h8hHEAAAAASUVORK5CYII=);
      }
      &.open {
        background-image: url(data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAADAAAAAwCAYAAABXAvmHAAAABmJLR0QA/wD/AP+gvaeTAAADrklEQVRoge2Xy2oUQRSG/+rbXDLTE0zIQmKiYFCUKIKIoEvFVdz5ACLoQ4jg1qUuFXyB4MaVm+DGlcSFlyCCcZGFEMGowUSc7q7jontS1d2nuntiJgj2gWGGmqm/vr/qnNM1QB111FFHHf9zCNMXm8vXLlqQjwHMlQh8jGBd988uvthzugrBGthcXphEIFYATFXU+WoJnOycf7q+d2jVwuIG5Za8J4NwSgYhKr4mgn745MfzK0f32wB7AhtLl78D6O0zCxdrRPLWxKWlZ6YfONyg7If/AjwAzAB4mLyzwRsIglEB7SYOFX1pMBD+9ap2sw1/bh6u30OcqRS/iAAARJQbA0iNE3a+D9fOEcVjPwnytjd7/8FIDVieh/ETZ2A3WyCKEsA0FAeKGFMzpRmOP3cg6A6AMgO7TCEBeP44/GOnYHkuSAYJaB4qDzrA1MaAnc+azoa+ZPkJCIHe8XmMTc/Ccpy0WCYNdCiSAQ+VO4W0EdYwtJQjelvBgDqBsZkj6MweTkFRKne53asAxekkm1KoQ3hXwYA6gebkJCgaGKKSBdTuGnavmk5ihNeRr0sNUBipH7TbIBmWLAC1QyZQBqpUR0+5RMcWqJBCYXwCwrFht7wkdbgF0gA86E4HyUFV1lEpt92YP/0JWCwzEJ9Aw/dBFCYaRR2EA1VzWFA95fKgrI4ErQhxV5aeACUn4HSS9DEWGQeah8qDluhkUkerq1T6GA0MTsDptnMFbF4gW6wZUGZOqY5eG0QgyKoG4hPwOq1cARdBmUEBMFBFxZoaU+NvKhkYdCG3244LeERQhQ9ERseWlHoGmA1EEeymB8sTmRpQYizoLqBKn9Jqznr3wssvFQ1IuH4HJMOCDsKBIlOYHOgQOpqWIOTyv9CA57eTyxhSQvwCCsAMCm3+kDqx0Vz+FxiI4PaaoCgsWSAPlQdFpgYUlGqlFXREvoUaDQCAO96MT4BdAHyr02sjM8eokzqRAh0r30KNBoQQcMe8uAYKi4wDVbtrBmV04l0x6UThr/77ygacTgOwJEhGxV2FgRrm7s/qJGtlntKrBxc+b1c24PZapX9I9uXun4yRoQMVGGik+v8ooNgHovlaMqSBrpvcgTgxBbC7PyQGHT3lsp2OJNtCjQacjp26A+3lH5JSHb3wkzFpRcOdgOWQJBlYbAfR68DYQTJjesoNey0R+DB9tb86lIHf37YeuQcaNy0LohgUqZ3kQdWcIa8lkgivgOiGEINJddRRRx111JGOPxu6cCKSl0emAAAAAElFTkSuQmCC);
      }
    }
  }
  .n-icon {
    float: left;
    font-size: 16px;
    line-height: 29px;
    margin: 5px 5px 0 10px;
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
  method: string;
  children: TreeItem[];
  expanded: boolean;
  parent: string;
  childIndex: number;
  hidden: boolean;
}

function getMethodColorType(method: string) {
  switch (method) {
    case HTTPMethod.DELETE:
      return "error";
      break;
    case HTTPMethod.PATCH:
    case HTTPMethod.PUT:
    case HTTPMethod.POST:
      return "success";
      break;
    default:
      return "info";
      break;
  }
}

function convertToTreeItems(params: {
  apiFolders: APIFolder[];
  apiSettings: APISetting[];
  expandedFolders: string[];
  topTreeItems: string[];
  keyword: string;
}): TreeItem[] {
  const { apiFolders, apiSettings, expandedFolders, topTreeItems } = params;
  const map = new Map<string, TreeItem>();

  const keyword = params.keyword.toLowerCase();

  const reg = /"method":"(\S+?)"/;
  apiSettings.forEach((item) => {
    let method = "";
    if (item.setting) {
      const result = reg.exec(item.setting);
      if (result?.length === 2) {
        method = result[1];
      }
    }
    map.set(item.id, {
      method,
      id: item.id,
      name: item.name,
      settingType: SettingType.HTTP,
      children: [],
      expanded: false,
      parent: "",
      childIndex: -1,
      hidden: false,
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
      hidden: false,
      method: "",
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
      if (!child || child === treeItem.id) {
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
  let result = [] as TreeItem[];
  map.forEach((item, key) => {
    if (children.includes(key)) {
      return;
    }
    result.push(item);
  });
  if (keyword) {
    const shouldBeHide = (item: TreeItem) => {
      let hidden = true;
      item.children.forEach((item) => {
        shouldBeHide(item);
        // 子元素有一个非hidden，则父元素非hidden
        if (!item.hidden) {
          hidden = false;
        }
      });
      if (item.name.toLowerCase().includes(keyword)) {
        hidden = false;
      }
      item.hidden = hidden;
    };
    result.forEach(shouldBeHide);
  }
  const filterVisible = (item: TreeItem) => {
    if (item.hidden) {
      return false;
    }
    item.children = item.children.filter(filterVisible);
    return true;
  };
  result = result.filter(filterVisible);
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
    const route = useRoute();
    const wrapper = ref(null);
    const collection = route.query.id as string;

    const collectionStore = useAPICollectionStore();
    const apiFolderStore = useAPIFolderStore();
    const apiSettingStore = useAPISettingStore();
    const { apiFolders } = storeToRefs(apiFolderStore);
    const { expandedFolders, topTreeItems } = storeToRefs(collectionStore);
    const { isDark } = storeToRefs(useSettingStore());
    const { apiSettings, selectedID } = storeToRefs(apiSettingStore);

    let currentTreeItems = [] as TreeItem[];
    let topTreeItemIDList = [] as string[];
    const renameItem = ref({
      name: "",
      id: "",
    });
    const renameValue = ref("");

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
        } else {
          apiSettingStore.select(item.id);
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
      let insertBefore = targetItem.id;
      // 如果是最后一个元素，而且bottom
      if (
        targetIndex === currentTreeItems.length - 1 &&
        overType === OverType.Bottom
      ) {
        insertBefore = "";
      }

      if (targetItem.settingType === SettingType.Folder) {
        // 拖动至文件上面，则add child
        if (overType === OverType.Over) {
          parentID = targetItem.id;
          insertBefore = "";
        } else {
          // 如果folder前面是元素，而且有parent
          // 则添加至该元素所有在folder
          const newTarget = currentTreeItems[targetIndex - 1];
          if (newTarget && newTarget.parent) {
            parentID = newTarget.parent;
            insertBefore = "";
          }
        }
      }
      try {
        await apiFolderStore.addChild({
          id: parentID,
          children: [moveItem.id],
          before: insertBefore,
        });
        // 设置至top items
        if (!parentID) {
          const index = topTreeItemIDList.indexOf(insertBefore);
          if (index === -1) {
            topTreeItemIDList.push(moveItem.id);
          } else {
            topTreeItemIDList.splice(index, 0, moveItem.id);
          }
          await collectionStore.updateTopTreeItems(
            collection,
            uniq(topTreeItemIDList)
          );
        }
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
      if (!isDragging && Math.abs(offset) > 5) {
        isDragging = true;
        nodeAddClass(wrapper.value, draggingClass);

        // 提交公共方法至html无法复制(TODO确认原因)
        // eslint-disable-next-line @typescript-eslint/ban-ts-comment
        // @ts-ignore
        moveTarget = target.cloneNode(true);
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

      handleMove(moveItemIndex, targetItemIndex, overType);
    };
    const handleMousedown = (e: MouseEvent) => {
      isDragging = false;
      // 无target或者点击非左键
      if (!e.currentTarget || e.button > 1) {
        return;
      }
      // TODO 此处导致无法复制，后续研究
      // e.preventDefault();
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

    const resetRename = () => {
      renameValue.value = "";
      renameItem.value = {
        name: "",
        id: "",
      };
    };

    const handelRename = async () => {
      // 无变化，无需修改
      const name = renameValue.value;
      const id = renameItem.value.id;
      if (!name || !id) {
        resetRename();
        return;
      }
      try {
        const folder = apiFolderStore.findByID(id);
        if (folder) {
          folder.name = name;
          await apiFolderStore.updateByID(id, folder);
        } else {
          const apiSetting = apiSettingStore.findByID(id);
          apiSetting.name = name;
          await apiSettingStore.updateByID(id, apiSetting);
        }
      } catch (err) {
        showError(message, err);
      } finally {
        resetRename();
      }
    };
    const handleKeydown = (e: KeyboardEvent) => {
      const key = e.key.toLowerCase();
      switch (key) {
        case "escape":
          {
            resetRename();
          }
          break;
        case "enter":
          {
            handelRename();
          }
          break;

        default:
          break;
      }
    };
    document.addEventListener("keydown", handleKeydown);
    onBeforeUnmount(() => {
      document.removeEventListener("keydown", handleKeydown);
    });

    return {
      renameValue,
      renameItem,
      selectedID,
      topTreeItems,
      expandedFolders,
      isDark,
      apiFolders,
      apiSettings,
      handleClick,
      handleMousedown,
      handelRename,
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
      topTreeItems,
      setTreeItems,
      selectedID,
      renameItem,
    } = this;
    const treeItems = convertToTreeItems({
      apiFolders,
      apiSettings,
      expandedFolders,
      topTreeItems,
      keyword,
    });
    const showAllChildren = keyword.trim().length !== 0;
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
        let folderClass = "folder";
        if (item.expanded) {
          folderClass += " open";
        } else {
          folderClass += " close";
        }
        let icon = <span class={folderClass}></span>;
        const isFolder = item.settingType === SettingType.Folder;
        if (!isFolder) {
          icon = (
            <NGradientText
              class="method"
              type={getMethodColorType(item.method)}
            >
              {item.method || HTTPMethod.GET}
            </NGradientText>
          );
        }

        const style = {
          "padding-left": `${level * 20}px`,
        };
        let cls = isDark ? "" : "light";
        if (item.id === selectedID) {
          cls += " selected";
        }
        if (item.id === renameItem.id) {
          cls += " renameItem";
        }
        const onClick =
          item.id !== selectedID
            ? (e: MouseEvent) => {
                if (this.renameItem.id) {
                  this.renameItem = {
                    id: "",
                    name: "",
                  };
                }
                const { target } = e;
                if (
                  nodeHasClass(target, "preventDefault") ||
                  nodeGetTagName(target) === "path"
                ) {
                  e.preventDefault();
                  return;
                }
                this.handleClick(item);
              }
            : undefined;
        const onDblclick = !isFolder
          ? (e: MouseEvent) => {
              if (!nodeHasClass(e.target, "name")) {
                return;
              }
              this.renameItem = {
                id: item.id,
                name: item.name,
              };
            }
          : undefined;
        currentTreeItems.push(item);
        itemList.push(
          <li
            key={`${item.id}-${level}`}
            data-index={treeItemIndex}
            class={cls}
            style={style}
            onDblclick={onDblclick}
            onClick={onClick}
            onMousedown={this.handleMousedown}
          >
            <APISettingTreeItemDropdown
              id={item.id}
              apiSettingType={item.settingType}
            />
            {icon}
            {item.id === renameItem.id && (
              <NInput
                class="renameInput"
                key={item.id}
                bordered={false}
                clearable
                defaultValue={renameItem.name}
                onVnodeMounted={(node) => {
                  node.el?.getElementsByTagName("input")[0]?.focus();
                }}
                onUpdateValue={(value) => {
                  this.renameValue = value;
                }}
                onInputBlur={() => {
                  this.handelRename();
                }}
              />
            )}
            {item.id !== renameItem.id && <span class="name">{item.name}</span>}
          </li>
        );
        treeItemIndex++;
        // 未展开的则不需要展示子元素
        // 而且非展示所有子元素
        if (!item.expanded && !showAllChildren) {
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

import { forEach } from "lodash-es";
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeRemove(node) {
  if (node && node.parentElement !== null) {
    node.parentElement.removeChild(node);
  }
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeInsertAt(fatherNode, node, position) {
  const refNode =
    position === 0
      ? fatherNode.children[0]
      : fatherNode.children[position - 1].nextSibling;
  fatherNode.insertBefore(node, refNode);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeInsertBefore(node, beforeNode) {
  const parent = beforeNode.parentNode;
  let found = false;
  // eslint-disable-next-line @typescript-eslint/ban-ts-comment
  // @ts-ignore
  forEach(parent.children, (item) => {
    if (found) {
      return;
    }
    if (item === beforeNode) {
      parent.insertBefore(node, item);
      found = true;
    }
  });
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeClone(node) {
  return node.nodeClone(true);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeSetStyle(node, style) {
  Object.assign(node.style, style);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeGetOffset(node) {
  if (!node) {
    return {
      left: 0,
      top: 0,
    };
  }
  const rect = node.getBoundingClientRect();
  return {
    left: rect.left,
    top: rect.top,
  };
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodGetScrollTop(node) {
  if (!node) {
    return 0;
  }
  return node.scrollTop;
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeAddClass(node, cls) {
  if (!node || !cls) {
    return;
  }
  node.classList.add(cls);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeHasClass(node, cls) {
  if (!node || !cls) {
    return false;
  }
  return node.classList.contains(cls);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeRemoveClass(node, cls) {
  if (!node || !cls) {
    return;
  }
  node.classList.remove(cls);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeGetOffsetHeightWidth(node) {
  if (!node) {
    return {
      height: 0,
      width: 0,
    };
  }
  return {
    height: node.offsetHeight,
    width: node.offsetWidth,
  };
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function nodeGetDataValue(node, key) {
  if (!node) {
    return "";
  }
  return node.dataset[key] as string;
}

export {
  nodeInsertAt,
  nodeRemove,
  nodeClone,
  nodeSetStyle,
  nodeInsertBefore,
  nodeGetOffset,
  nodGetScrollTop,
  nodeAddClass,
  nodeRemoveClass,
  nodeHasClass,
  nodeGetOffsetHeightWidth,
  nodeGetDataValue,
};

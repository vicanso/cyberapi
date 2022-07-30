import { forEach } from "lodash-es";
// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function removeNode(node) {
  if (node && node.parentElement !== null) {
    node.parentElement.removeChild(node);
  }
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function insertNodeAt(fatherNode, node, position) {
  const refNode =
    position === 0
      ? fatherNode.children[0]
      : fatherNode.children[position - 1].nextSibling;
  fatherNode.insertBefore(node, refNode);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function insertNodeBefore(node, beforeNode) {
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
function cloneNode(node) {
  return node.cloneNode(true);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function setNodeStyle(node, style) {
  Object.assign(node.style, style);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function getNodeOffset(node) {
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
function getNodeScrollTop(node) {
  if (!node) {
    return 0;
  }
  return node.scrollTop;
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function addNodeClass(node, cls) {
  if (!node || !cls) {
    return;
  }
  node.classList.add(cls);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function removeNodeClass(node, cls) {
  if (!node || !cls) {
    return;
  }
  node.classList.remove(cls);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function getNodeOffsetHeightWidth(node) {
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
function getNodeDataValue(node, key) {
  if (!node) {
    return "";
  }
  return node.dataset[key] as string;
}

export {
  insertNodeAt,
  removeNode,
  cloneNode,
  setNodeStyle,
  insertNodeBefore,
  getNodeOffset,
  getNodeScrollTop,
  addNodeClass,
  removeNodeClass,
  getNodeOffsetHeightWidth,
  getNodeDataValue,
};

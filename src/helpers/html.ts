// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function removeNode(node) {
  if (node.parentElement !== null) {
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
function cloneNode(node) {
  return node.cloneNode(true);
}

// eslint-disable-next-line @typescript-eslint/ban-ts-comment
// @ts-ignore
function setNodeStyle(node, style) {
  Object.assign(node.style, style);
}

export { insertNodeAt, removeNode, cloneNode, setNodeStyle };

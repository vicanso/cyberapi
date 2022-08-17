import { defineComponent, PropType, StyleValue } from "vue";
import { css } from "@linaria/core";
import { NDivider } from "naive-ui";

import { nodeInsertAt, nodeSetStyle, nodeRemove } from "../helpers/html";

const dividerClass = css`
  margin: 0 !important;
  height: 100% !important;
  position: absolute;
`;

export default defineComponent({
  name: "ExColumn",
  props: {
    width: {
      type: Number,
      default: 0,
    },
    left: {
      type: Number,
      default: 0,
    },
    showDivider: {
      type: Boolean,
      default: false,
    },
    onResize: {
      type: Function as PropType<(value: number) => void>,
      default: () => {
        return (value: number) => {
          console.info(value);
        };
      },
    },
  },
  setup(props) {
    let isDragging = false;
    let originClientX = 0;
    let target: EventTarget;
    let moveLeft = 0;
    const onMousemove = (e: MouseEvent) => {
      if (!isDragging) {
        return;
      }
      e.preventDefault();
      moveLeft = e.clientX - originClientX;
      nodeSetStyle(target, {
        left: `${e.clientX}px`,
      });
    };
    const onMouseup = () => {
      if (props.onResize) {
        props.onResize(moveLeft);
      }
      moveLeft = 0;
      isDragging = false;
      document.removeEventListener("mousemove", onMousemove);
      document.removeEventListener("mouseup", onMouseup);
      if (target) {
        nodeRemove(target);
      }
    };
    const onMousedown = (e: MouseEvent) => {
      isDragging = false;
      if (!e.currentTarget) {
        return;
      }

      originClientX = e.clientX;
      isDragging = true;
      document.addEventListener("mousemove", onMousemove);
      document.addEventListener("mouseup", onMouseup);

      // 提交公共方法至html无法复制(TODO确认原因)
      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      target = e.currentTarget.cloneNode(true);

      nodeSetStyle(target, {
        left: `${originClientX}px`,
        width: "2px",
        zIndex: "9",
      });
      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      nodeInsertAt(e.currentTarget.parentNode.parentNode, target, 0);
    };

    return {
      onMousedown,
    };
  },
  render() {
    const { left, width, showDivider } = this.$props;
    const { $slots } = this;
    const style: StyleValue = {
      position: "absolute",
      top: "0px",
      bottom: "0px",
      left: `${left}px`,
      width: `${width}px`,
      overflow: "hidden",
    };
    if (!width) {
      delete style.width;
      style.right = "0px";
    }
    const divider = showDivider && (
      <div
        style={{
          position: "absolute",
          cursor: "col-resize",
          padding: "0px 2px",
          height: "100%",
        }}
        onMousedown={this.onMousedown}
      >
        <NDivider vertical class={dividerClass} />
      </div>
    );
    return (
      <div style={style}>
        {divider}
        {$slots.default && $slots.default()}
      </div>
    );
  },
});

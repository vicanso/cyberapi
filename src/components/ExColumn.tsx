import { defineComponent, PropType, StyleValue } from "vue";
import { css } from "@linaria/core";
import { NDivider } from "naive-ui";

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
    let originPageX = 0;
    let target: EventTarget;
    let moveLeft = 0;
    const onMousemove = (e: MouseEvent) => {
      if (!isDragging) {
        return;
      }
      e.preventDefault();
      moveLeft = e.pageX - originPageX;
      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      target.style.left = `${moveLeft}px`;
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
        // eslint-disable-next-line @typescript-eslint/ban-ts-comment
        // @ts-ignore
        target.parentElement.removeChild(target);
      }
    };
    const onMousedown = (e: MouseEvent) => {
      if (!e.currentTarget) {
        return;
      }

      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      target = e.currentTarget.cloneNode(true);
      // eslint-disable-next-line @typescript-eslint/ban-ts-comment
      // @ts-ignore
      e.currentTarget.parentNode.insertBefore(target, e.currentTarget);

      originPageX = e.pageX;
      isDragging = true;
      document.addEventListener("mousemove", onMousemove);
      document.addEventListener("mouseup", onMouseup);
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

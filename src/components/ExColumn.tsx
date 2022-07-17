import { defineComponent, PropType, StyleValue, ref } from "vue";
import { css } from "@linaria/core";
import { NDivider } from "naive-ui";

const dividerClass = css`
  margin: 0 !important;
  height: 100% !important;
  position: absolute;
  cursor: col-resize;
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
  setup() {
    let isDragging = false;
    let originPageX = 0;
    const moveLeft = ref(0);
    const onMousemove = (e: MouseEvent) => {
      if (!isDragging) {
        return;
      }
      e.preventDefault();
      moveLeft.value = e.pageX - originPageX;
    };
    const onMouseup = () => {
      isDragging = false;
      document.removeEventListener("mousemove", onMousemove);
      document.removeEventListener("mouseup", onMouseup);
    };
    const onMousedown = (e: MouseEvent) => {
      if (!e.currentTarget) {
        return;
      }
      originPageX = e.pageX;
      isDragging = true;
      document.addEventListener("mousemove", onMousemove);
      document.addEventListener("mouseup", onMouseup);
    };

    return {
      moveLeft,
      onMousedown,
    };
  },
  render() {
    const { left, width, showDivider } = this.$props;
    const { $slots, moveLeft } = this;
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
      <div onMousedown={this.onMousedown}>
        <NDivider
          style={{
            left: `${moveLeft}px`,
          }}
          vertical
          class={dividerClass}
        />
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

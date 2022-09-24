import { defineComponent, onBeforeUnmount, ref } from "vue";

import { css } from "@linaria/core";

const timerClass = css`
  font-variant-numeric: tabular-nums;
  font-weight: 400;
`;

export default defineComponent({
  name: "ExTimer",
  setup() {
    const now = Date.now();
    const seconds = ref(0);
    const timer = setInterval(() => {
      seconds.value = Date.now() - now;
    }, 100);
    onBeforeUnmount(() => clearInterval(timer));
    return {
      seconds,
    };
  },
  render() {
    const { seconds } = this;
    return <span class={timerClass}>{(seconds / 1000).toFixed(1)} s</span>;
  },
});

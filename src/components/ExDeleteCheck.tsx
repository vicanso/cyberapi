import { defineComponent, ref, PropType } from "vue";
import { NButton, NIcon } from "naive-ui";
import { AlertCircleOutline, TrashOutline } from "@vicons/ionicons5";
import { css } from "@linaria/core";

const checkClass = css`
  color: #f2c97d;
  font-weight: 900;
`;

// 两次短时间点击则清除

export default defineComponent({
  name: "ExDeleteCheck",
  props: {
    onConfirm: {
      type: Function as PropType<() => void>,
      required: true,
    },
  },
  setup(props) {
    const deleting = ref(false);
    const handleClick = () => {
      if (!deleting.value) {
        deleting.value = true;
      } else if (props.onConfirm) {
        props.onConfirm();
      }
    };

    return {
      handleClick,
      deleting,
    };
  },
  render() {
    const { deleting } = this;
    return (
      <NButton
        quaternary
        onClick={() => {
          this.handleClick();
        }}
      >
        <NIcon class={deleting && checkClass}>
          {!deleting && <TrashOutline />}
          {deleting && <AlertCircleOutline />}
        </NIcon>
      </NButton>
    );
  },
});

// API应用配置列表
import { css } from "@linaria/core";
import { defineComponent, ref } from "vue";

import APISettingTreeHeader from "./header";
import APISettingTreeItems from "./items";

const treesClass = css`
  padding: 10px;
`;

export default defineComponent({
  name: "APISettingTree",
  setup() {
    const keyword = ref("");
    return {
      keyword,
    };
  },
  render() {
    return (
      <div class={treesClass}>
        <APISettingTreeHeader
          onFilter={(value: string) => {
            this.keyword = value;
          }}
        />
        <APISettingTreeItems />
      </div>
    );
  },
});

// API应用配置列表
import { css } from "@linaria/core";
import { defineComponent, ref } from "vue";

import APISettingTreesHeader from "./header";

const treesClass = css`
  padding: 10px;
`;

export default defineComponent({
  name: "APISettingTrees",
  setup() {
    const keyword = ref("");
    return {
      keyword,
    };
  },
  render() {
    return (
      <div class={treesClass}>
        <APISettingTreesHeader
          onFilter={(value: string) => {
            this.keyword = value;
          }}
        />
      </div>
    );
  },
});

import { defineComponent } from "vue";
import { css } from "@linaria/core";
import { NButton, NGradientText } from "naive-ui";

import { MAIN_CONTENT_WIDTH } from "../constant";

const headerClass = css`
  height: 48px;
  line-height: 48px;
  padding: 20px 0;
  font-size: 18px;
  font-weight: 900;
  .wrapper {
    max-width: ${MAIN_CONTENT_WIDTH}px;
    margin: auto;
  }
  .name {
    font-size: 32px;
  }
  .functions {
    font-size: 14px;
    float: right;
    margin-right: 10px;
  }
`;

export default defineComponent({
  name: "AppHeader",
  render() {
    return (
      <header class={headerClass}>
        <div class="wrapper">
          <NGradientText class="name" type="warning">
            Cyber API
          </NGradientText>
          <div class="functions">
            <NButton text>下载APP</NButton>
          </div>
        </div>
      </header>
    );
  },
});

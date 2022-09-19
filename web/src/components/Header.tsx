import { defineComponent } from "vue";
import { css } from "@linaria/core";
import { NButton, NGradientText } from "naive-ui";
import { RouterLink } from "vue-router";

import { MAIN_CONTENT_WIDTH } from "../constant";
import { goToDoc } from "../router";
import { names } from "../router/routes";

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
    .n-button {
      margin-left: 15px;
    }
  }
`;

export default defineComponent({
  name: "AppHeader",
  render() {
    return (
      <header class={headerClass}>
        <div class="wrapper">
          <RouterLink
            to={{
              name: names.home,
            }}
          >
            <NGradientText
              class="name"
              type="warning"
              gradient={{
                deg: 90,
                from: "#7cb0fa",
                to: "#ae3ff5",
              }}
            >
              Cyber API
            </NGradientText>
          </RouterLink>
          <div class="functions">
            <NButton
              text
              onClick={() => {
                goToDoc("start");
              }}
            >
              开始使用
            </NButton>
            <NButton
              text
              onClick={() => {
                window.location.href =
                  "https://github.com/vicanso/cyberapi/releases";
              }}
            >
              下载APP
            </NButton>
          </div>
        </div>
      </header>
    );
  },
});

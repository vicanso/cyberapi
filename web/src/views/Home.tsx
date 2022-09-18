import { defineComponent } from "vue";
import { css } from "@linaria/core";
import { NP, NText, NButton, NGi, NGrid, NH3 } from "naive-ui";

import { MAIN_CONTENT_WIDTH } from "../constant";

const appClass = css`
  width: ${MAIN_CONTENT_WIDTH}px;
  margin: auto;
  .content {
    height: 350px;
    padding-right: 630px;
    background-image: url(/assets/main.png);
    background-size: 600px;
    background-repeat: no-repeat;
    background-position: top right;
    .title {
      font-size: 18px;
    }
    .large {
      font-size: 32px;
    }
    .functions {
      margin-top: 15px;
      text-align: right;
      .n-button {
        padding: 0 25px;
      }
    }
  }
  .desc {
    background-color: rgba(47, 47, 47, 255);
    padding: 20px 30px 30px 30px;
    border-radius: 8px;
  }
`;

const descriptions = [
  {
    title: "开源免费",
    desc: "代码开源托管在github，使用Apache 2.0开源协议",
  },
  {
    title: "跨平台",
    desc: "基于Tauri开发，所有主流的PC系统均可使用，一致的客户端便于团队协作统一",
  },
  {
    title: "更小的体积",
    desc: "Macos与windows使用自带的浏览器组件，避免打包chrome，安装包的体积均在10MB以下",
  },
  {
    title: "更低的资源占用",
    desc: "更少的内存占用，更高效的执行效率，运行期间占用内存基本在200MB左右，程序瞬间启动",
  },
];

export default defineComponent({
  name: "HomeView",
  render() {
    const items = descriptions.map((item) => {
      return (
        <NGi span={8} key={item.title} class="desc">
          <NH3>{item.title}</NH3>
          <NText>{item.desc}</NText>
        </NGi>
      );
    });
    return (
      <div class={appClass}>
        <div class="content">
          <NP class="title">CyberAPI是基于tauri开发的跨平台API客户端 </NP>
          <h3 class="large">更快 更小 更简单</h3>
          <NText>
            用于开发测试HTTP的接口，提供macos、windows与linux三个系统的安装包，安装包均在10MB以下，启动时间在1秒以下，性能表现也挺不错。可直接导入postname与insomnia的配置，零成本快速迁移。
          </NText>
          <div class="functions">
            <NButton>下载</NButton>
          </div>
        </div>
        <NGrid xGap={20} yGap={20}>
          {items}
        </NGrid>
      </div>
    );
  },
});

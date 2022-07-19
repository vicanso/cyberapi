import {
  NBreadcrumb,
  NBreadcrumbItem,
  NDivider,
  NGrid,
  NGi,
  NButton,
  NIcon,
  NModal,
} from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent, ref } from "vue";
import { storeToRefs } from "pinia";
import { SettingsOutline } from "@vicons/ionicons5";

import { mainHeaderHeight } from "../constants/style";
import { i18nCommon } from "../i18n";
import { names } from "../router/routes";
import { goTo } from "../router";
import { useHeaderStore } from "../stores/header";
import AppSetting from "./AppSetting";

const headerClass = css`
  height: ${mainHeaderHeight}px;
  line-height: ${mainHeaderHeight}px;
`;

const logoClass = css`
  margin-left: 15px;
  margin-right: 10px;
  float: left;
  font-weight: 600;
  div {
    margin-left: 20px !important;
  }
`;
const breadcrumbClass = css`
  padding-top: ${(mainHeaderHeight - 25) / 2}px;
`;
const functionsClass = css`
  text-align: right;
  margin-right: 10px;
  .n-button {
    margin-top: 8px;
  }
`;

export default defineComponent({
  name: "AppHeaderView",
  setup() {
    const headerStore = useHeaderStore();

    const showSetting = ref(false);

    const { breadcrumbs } = storeToRefs(headerStore);
    return {
      breadcrumbs,
      showSetting,
    };
  },
  render() {
    const { breadcrumbs, $route, showSetting } = this;
    const arr = [
      {
        route: names.home,
        name: i18nCommon("dashboard"),
      },
    ];
    arr.push(...breadcrumbs);
    const items = arr.map((item) => {
      return (
        <NBreadcrumbItem
          key={item.route}
          onClick={() => {
            if (item.route === $route.name) {
              return;
            }
            goTo(item.route);
          }}
        >
          {item.name}
        </NBreadcrumbItem>
      );
    });

    const modal = (
      <NModal
        show={showSetting}
        closeOnEsc
        onEsc={() => {
          this.showSetting = false;
        }}
        onMaskClick={() => {
          this.showSetting = false;
        }}
      >
        <AppSetting />
      </NModal>
    );

    return (
      <div>
        {modal}
        <header class={headerClass}>
          <NGrid>
            <NGi span={12}>
              <div class={logoClass}>
                {i18nCommon("app")}
                <NDivider vertical />
              </div>
              <NBreadcrumb class={breadcrumbClass}>{items}</NBreadcrumb>
            </NGi>
            <NGi span={12}>
              <div class={functionsClass}>
                <NButton
                  quaternary
                  onClick={() => {
                    this.showSetting = true;
                  }}
                >
                  <NIcon class="font20 bold">
                    <SettingsOutline />
                  </NIcon>
                </NButton>
              </div>
            </NGi>
          </NGrid>
        </header>
      </div>
    );
  },
});

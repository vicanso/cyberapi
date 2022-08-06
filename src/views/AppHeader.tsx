import {
  NBreadcrumb,
  NBreadcrumbItem,
  NDivider,
  NGrid,
  NGi,
  NButton,
  NIcon,
  NTabs,
  NTab,
} from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent } from "vue";
import { storeToRefs } from "pinia";
import { BowlingBallOutline, SettingsOutline } from "@vicons/ionicons5";

import { mainHeaderHeight } from "../constants/style";
import { i18nCommon } from "../i18n";
import { names } from "../router/routes";
import { goTo } from "../router";
import { useHeaderStore } from "../stores/header";
import { useDialogStore } from "../stores/dialog";

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
    margin-left: 5px;
  }
`;

export default defineComponent({
  name: "AppHeaderView",
  setup() {
    const headerStore = useHeaderStore();
    const dialogStore = useDialogStore();

    const { breadcrumbs } = storeToRefs(headerStore);
    const showSettingDialog = () => {
      dialogStore.toggleSettingDialog(true);
    };
    const showCookieDialog = () => {
      dialogStore.toggleCookieDialog(true);
    };
    return {
      breadcrumbs,
      showCookieDialog,
      showSettingDialog,
    };
  },
  render() {
    const { breadcrumbs, $route, showSettingDialog, showCookieDialog } = this;
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

    return (
      <div>
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
                    showCookieDialog();
                  }}
                >
                  <NIcon class="font20 bold">
                    <BowlingBallOutline />
                  </NIcon>
                </NButton>
                <NButton
                  quaternary
                  onClick={() => {
                    showSettingDialog();
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

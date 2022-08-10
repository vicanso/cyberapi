import {
  NBreadcrumb,
  NBreadcrumbItem,
  NDivider,
  NGrid,
  NGi,
  NButton,
  NIcon,
} from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent, watch, ref } from "vue";
import { storeToRefs } from "pinia";
import {
  BowlingBallOutline,
  CodeSlashOutline,
  SettingsOutline,
} from "@vicons/ionicons5";
import { useRoute } from "vue-router";

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
  .n-icon {
    font-size: 20px;
    font-weight: 900;
  }
`;

export default defineComponent({
  name: "AppHeaderView",
  setup() {
    const headerStore = useHeaderStore();
    const dialogStore = useDialogStore();
    const route = useRoute();

    const currentRoute = ref(route.name);
    watch(
      () => route.name,
      (value) => {
        currentRoute.value = value;
      }
    );

    const { breadcrumbs } = storeToRefs(headerStore);
    const showSettingDialog = () => {
      dialogStore.toggleSettingDialog(true);
    };
    const showCookieDialog = () => {
      dialogStore.toggleCookieDialog(true);
    };
    const showEnvironmentDialog = () => {
      dialogStore.toggleEnvironmentDialog(true);
    };
    return {
      currentRoute,
      breadcrumbs,
      showCookieDialog,
      showSettingDialog,
      showEnvironmentDialog,
    };
  },
  render() {
    const {
      breadcrumbs,
      $route,
      showSettingDialog,
      showCookieDialog,
      showEnvironmentDialog,
      currentRoute,
    } = this;
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
                {currentRoute == names.collection && (
                  <NButton
                    quaternary
                    onClick={() => {
                      showEnvironmentDialog();
                    }}
                  >
                    <NIcon>
                      <CodeSlashOutline />
                    </NIcon>
                  </NButton>
                )}
                <NButton
                  quaternary
                  onClick={() => {
                    showCookieDialog();
                  }}
                >
                  <NIcon>
                    <BowlingBallOutline />
                  </NIcon>
                </NButton>
                <NButton
                  quaternary
                  onClick={() => {
                    showSettingDialog();
                  }}
                >
                  <NIcon>
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

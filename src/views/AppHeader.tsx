import {
  NBreadcrumb,
  NBreadcrumbItem,
  NDivider,
  NButton,
  NIcon,
  NTab,
  NTabs,
} from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent, watch, ref, StyleValue } from "vue";
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
import { useSettingStore } from "../stores/setting";

const logoWrapperWidth = 300;

const headerClass = css`
  height: ${mainHeaderHeight}px;
  line-height: ${mainHeaderHeight}px;
  .logoWrapper {
    float: left;
    width: ${logoWrapperWidth}px;
  }
  .logo {
    margin-left: 15px;
    margin-right: 10px;
    float: left;
    font-weight: 600;
    div {
      margin-left: 20px !important;
    }
  }
  .breadcrumb {
    padding-top: ${(mainHeaderHeight - 25) / 2}px;
  }
  .latestApis {
    float: left;
    padding-top: 7px;
  }
  .funcs {
    float: right;
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
  }
`;

export default defineComponent({
  name: "AppHeaderView",
  setup() {
    const headerStore = useHeaderStore();
    const dialogStore = useDialogStore();
    const route = useRoute();
    const settingStore = useSettingStore();

    const { collectionColumnWidths } = storeToRefs(settingStore);

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
      collectionColumnWidths,
      currentRoute,
      breadcrumbs,
      showCookieDialog,
      showSettingDialog,
      showEnvironmentDialog,
    };
  },
  render() {
    const {
      collectionColumnWidths,
      breadcrumbs,
      $route,
      showSettingDialog,
      showCookieDialog,
      showEnvironmentDialog,
      currentRoute,
    } = this;
    console.dir(collectionColumnWidths[0]);
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
    const logoWrapperStyle: StyleValue = {};
    if (
      collectionColumnWidths.length &&
      collectionColumnWidths[0] > logoWrapperWidth
    ) {
      logoWrapperStyle["width"] = `${collectionColumnWidths[0] + 2}px`;
    }

    return (
      <div>
        <header class={headerClass}>
          <div class="logoWrapper" style={logoWrapperStyle}>
            <div class="logo">
              {i18nCommon("app")}
              <NDivider vertical />
            </div>
            <NBreadcrumb class="breadcrumb">{items}</NBreadcrumb>
          </div>
          {/* TODO 添加最近使用的接口 */}
          <div class="latestApis">
            <NTabs
              type="card"
              closable
              tabStyle={{
                "min-width": "100px",
              }}
              defaultValue={"abc"}
            >
              <NTab name="abc"></NTab>
              <NTab name="abc"></NTab>
            </NTabs>
          </div>

          <div class="funcs">
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
        </header>
      </div>
    );
  },
});

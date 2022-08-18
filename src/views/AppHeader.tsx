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
import { defineComponent, watch, ref, StyleValue, onBeforeUnmount } from "vue";
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
import { usePinRequestStore } from "../stores/pin_request";
import { useAPISettingStore } from "../stores/api_setting";

const logoWidth = 300;

const headerClass = css`
  position: relative;
  height: ${mainHeaderHeight}px;
  line-height: ${mainHeaderHeight}px;
  .loggWrapper {
    float: left;
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
  .pinApis {
    left: ${logoWidth}px;
    position: absolute;
    right: 180px;
    padding-top: 12px;
    .n-tabs-pad {
      border-bottom: none !important;
    }
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

    const apiSettingStore = useAPISettingStore();
    const pinRequestStore = usePinRequestStore();

    const { requests } = storeToRefs(pinRequestStore);

    const activePinRequest = ref("");
    let activePinRequestID = "";

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

    const stop = watch(
      () => apiSettingStore.selectedID,
      (id) => {
        // 如果不是当前的tab，是置空
        if (id !== activePinRequestID) {
          activePinRequest.value = "";
        }
      }
    );
    onBeforeUnmount(() => {
      stop();
    });

    const getRequest = (name: string) => {
      const arr = name.split(":");
      const index = Number(arr[0]) - 1;
      return pinRequestStore.requests[index];
    };

    const handleSelectePinRequest = (name: string) => {
      const req = getRequest(name);
      if (!req) {
        return;
      }
      apiSettingStore.select(req.id);
      activePinRequest.value = name;
      activePinRequestID = req.id;
    };
    const handleRemovePinRequest = (name: string) => {
      const req = getRequest(name);
      if (!req) {
        return;
      }
      pinRequestStore.remove(req.id);
    };
    return {
      requests,
      activePinRequest,
      collectionColumnWidths,
      currentRoute,
      breadcrumbs,
      showCookieDialog,
      showSettingDialog,
      showEnvironmentDialog,
      handleSelectePinRequest,
      handleRemovePinRequest,
      findByID: apiSettingStore.findByID,
    };
  },
  render() {
    const {
      requests,
      collectionColumnWidths,
      breadcrumbs,
      $route,
      showSettingDialog,
      showCookieDialog,
      showEnvironmentDialog,
      findByID,
      currentRoute,
      activePinRequest,
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
    const pinApisStyle: StyleValue = {};
    if (
      collectionColumnWidths.length &&
      collectionColumnWidths[0] > logoWidth
    ) {
      pinApisStyle["left"] = `${collectionColumnWidths[0] + 2}px`;
    }

    const getTabs = () => {
      return requests.map((item, index) => {
        const result = findByID(item.id);
        if (!result) {
          return;
        }
        const name = `${index + 1}: ${result.name}`;
        return <NTab name={name} key={name}></NTab>;
      });
    };

    return (
      <div>
        <header class={headerClass}>
          <div class="loggWrapper">
            <div class="logo">
              {i18nCommon("app")}
              <NDivider vertical />
            </div>
            <NBreadcrumb class="breadcrumb">{items}</NBreadcrumb>
          </div>
          {currentRoute == names.collection && (
            <div class="pinApis" style={pinApisStyle}>
              <NTabs
                type="card"
                closable
                tabStyle={{
                  "min-width": "100px",
                  "border-bottom": "none",
                }}
                defaultValue={""}
                value={activePinRequest}
                onClose={(value) => {
                  this.handleRemovePinRequest(value);
                }}
                onUpdateValue={(value: string) => {
                  this.handleSelectePinRequest(value);
                }}
              >
                {getTabs()}
              </NTabs>
            </div>
          )}

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

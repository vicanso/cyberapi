import {
  NBreadcrumb,
  NBreadcrumbItem,
  NDivider,
  NIcon,
  NTab,
  NTabs,
  NDropdown,
  NSpace,
  useMessage,
} from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent, watch, ref, StyleValue, onBeforeUnmount } from "vue";
import { storeToRefs } from "pinia";
import {
  AppsOutline,
  BowlingBallOutline,
  CodeSlashOutline,
  LanguageOutline,
  ServerOutline,
  SettingsOutline,
} from "@vicons/ionicons5";
import { useRoute } from "vue-router";

import { mainHeaderHeight } from "../constants/style";
import { i18nCommon, i18nSetting, LANG } from "../i18n";
import { names } from "../router/routes";
import { goTo } from "../router";
import { useHeaderStore } from "../stores/header";
import { useDialogStore } from "../stores/dialog";
import { useSettingStore } from "../stores/setting";
import { usePinRequestStore } from "../stores/pin_request";
import { useAPISettingStore } from "../stores/api_setting";
import { setLang } from "../stores/local";
import { showError } from "../helpers/util";

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
    right: 120px;
    padding-top: 12px;
    .n-tabs-pad {
      border-bottom: none !important;
    }
  }
  .funcs {
    float: right;
    padding-top: 6px;
    margin-right: 20px;
    text-align: right;
    .n-icon {
      padding: 5px;
      font-size: 20px;
      font-weight: 900;
    }
  }
`;

enum FnKey {
  cookie = "cookie",
  store = "store",
  env = "env",
  setting = "setting",
}

export default defineComponent({
  name: "AppHeaderView",
  setup() {
    const message = useMessage();
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
    const stop = watch(
      () => apiSettingStore.selectedID,
      (id) => {
        // 如果不是当前的tab，是置空
        if (id !== activePinRequestID) {
          activePinRequest.value = "";
          pinRequestStore.requests.forEach((item, index) => {
            if (item.id === id) {
              const result = apiSettingStore.findByID(id);
              if (!result) {
                return;
              }
              activePinRequest.value = `${index + 1}: ${result.name}`;
            }
          });
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
    const handleFunction = (key: string) => {
      switch (key) {
        case FnKey.cookie:
          dialogStore.toggleCookieDialog(true);
          break;
        case FnKey.store:
          dialogStore.toggleStoreDialog(true);
          break;
        case FnKey.env:
          dialogStore.toggleEnvironmentDialog(true);
          break;
        case FnKey.setting:
          dialogStore.toggleSettingDialog(true);
          break;
        default:
          break;
      }
    };

    const handleChangeLang = async (lang: string) => {
      try {
        await setLang(lang);
        message.info(i18nSetting("langChangeSuccess"));
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      requests,
      activePinRequest,
      collectionColumnWidths,
      currentRoute,
      breadcrumbs,
      handleFunction,
      handleSelectePinRequest,
      handleRemovePinRequest,
      handleChangeLang,
      findByID: apiSettingStore.findByID,
    };
  },
  render() {
    const {
      requests,
      collectionColumnWidths,
      breadcrumbs,
      $route,
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

    const options = [
      {
        label: i18nSetting("cookieSetting"),
        key: FnKey.cookie,
        icon: () => (
          <NIcon>
            <BowlingBallOutline />
          </NIcon>
        ),
      },
      {
        label: i18nSetting("storeSetting"),
        key: FnKey.store,
        icon: () => (
          <NIcon>
            <ServerOutline />
          </NIcon>
        ),
      },
      {
        label: i18nSetting("appSetting"),
        key: FnKey.setting,
        icon: () => (
          <NIcon>
            <SettingsOutline />
          </NIcon>
        ),
      },
    ];
    if (currentRoute == names.collection) {
      options.unshift({
        label: i18nSetting("envSetting"),
        key: FnKey.env,
        icon: () => (
          <NIcon>
            <CodeSlashOutline />
          </NIcon>
        ),
      });
    }

    const langs = [
      {
        label: "中文",
        key: LANG.zh,
      },
      {
        label: "English",
        key: LANG.en,
      },
    ];

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
            <NSpace>
              <NDropdown
                options={options}
                onSelect={(key: string) => {
                  this.handleFunction(key);
                }}
              >
                <NIcon>
                  <AppsOutline />
                </NIcon>
              </NDropdown>
              <NDropdown
                options={langs}
                onSelect={(key: string) => {
                  this.handleChangeLang(key);
                }}
              >
                <NIcon>
                  <LanguageOutline />
                </NIcon>
              </NDropdown>
            </NSpace>
          </div>
        </header>
      </div>
    );
  },
});

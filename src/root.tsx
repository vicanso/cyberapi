import { defineComponent, onBeforeMount, ref } from "vue";
import { css } from "@linaria/core";
import {
  darkTheme,
  NConfigProvider,
  NDialogProvider,
  NGlobalStyle,
  NLoadingBarProvider,
  NMessageProvider,
  NNotificationProvider,
} from "naive-ui";
import { storeToRefs } from "pinia";
import { closeSplashscreen } from "./commands/window";
import { useCommonStore } from "./stores/common";
import App from "./App";
import ExLoading from "./components/ExLoading";
import { doHTTPRequest } from "./commands/http_request";
import { listCookie } from "./commands/cookies";

const loadingClass = css`
  margin-top: 50px;
`;

export default defineComponent({
  name: "RootView",
  setup() {
    const commonStore = useCommonStore();
    const { setting } = storeToRefs(commonStore);
    const processing = ref(true);
    onBeforeMount(async () => {
      await commonStore.getSetting();
      processing.value = false;
      closeSplashscreen();
      doHTTPRequest({
        method: "GET",
        uri: "https://store.gf.com.cn/rest/user/session",
        body: "",
      })
        .then(console.dir)
        .catch(console.error);
      listCookie().then(console.dir).catch(console.error);
    });

    return {
      processing,
      setting,
    };
  },
  render() {
    const { processing, setting } = this;
    if (processing) {
      return <ExLoading class={loadingClass} />;
    }
    const { isDark } = setting;
    return (
      <NConfigProvider theme={isDark ? darkTheme : null}>
        <NLoadingBarProvider>
          <NMessageProvider>
            <NNotificationProvider>
              <NDialogProvider>
                <App />
              </NDialogProvider>
            </NNotificationProvider>
          </NMessageProvider>
        </NLoadingBarProvider>
        <NGlobalStyle />
      </NConfigProvider>
    );
  },
});

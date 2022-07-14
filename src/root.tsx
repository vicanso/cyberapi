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
      // 延时1秒关闭，便于程序初始化
      setTimeout(closeSplashscreen, 1000);
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
        <NGlobalStyle />
        <NLoadingBarProvider>
          <NMessageProvider>
            <NNotificationProvider>
              <NDialogProvider>
                <App />
              </NDialogProvider>
            </NNotificationProvider>
          </NMessageProvider>
        </NLoadingBarProvider>
      </NConfigProvider>
    );
  },
});

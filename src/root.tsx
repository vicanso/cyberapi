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
import { useSettingStore } from "./stores/setting";
import App from "./App";
import ExLoading from "./components/ExLoading";

export default defineComponent({
  name: "RootView",
  setup() {
    const settingStore = useSettingStore();
    const { isDark } = storeToRefs(settingStore);
    const processing = ref(true);
    onBeforeMount(async () => {
      await settingStore.fetch();
      processing.value = false;
      // 延时1秒关闭，便于程序初始化
      setTimeout(closeSplashscreen, 1000);
    });

    return {
      processing,
      isDark,
    };
  },
  render() {
    const { processing, isDark } = this;
    if (processing) {
      return <ExLoading />;
    }
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

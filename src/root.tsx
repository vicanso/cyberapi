import { defineComponent, onBeforeMount, ref } from "vue";
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
import { useAppStore } from "./stores/app";

export default defineComponent({
  name: "RootView",
  setup() {
    const settingStore = useSettingStore();
    const appStore = useAppStore();
    const { isDark } = storeToRefs(settingStore);
    const processing = ref(true);

    // 避免发布版本可以reload页面
    if (window.location.protocol.includes("tauri")) {
      document.addEventListener("contextmenu", (e) => e.preventDefault());
    }
    const startedAt = Date.now();
    onBeforeMount(async () => {
      try {
        await appStore.fetch();
        await settingStore.fetch();
        await settingStore.resize();
      } catch (err) {
        // TODO 初始化基本不会出错，是否有其它方法提示
        console.error(err);
      } finally {
        processing.value = false;
        // splashscreen最多300ms
        const delay = 300 - (Date.now() - startedAt);
        setTimeout(closeSplashscreen, delay);
      }
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

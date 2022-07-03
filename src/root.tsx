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
import { useCommonStore } from "./stores/common";
import App from "./App";

export default defineComponent({
  name: "Root",
  setup() {
    // console.dir(i18n.global.t);``
    const commonStore = useCommonStore();
    const { setting } = storeToRefs(commonStore);
    const processing = ref(true);
    onBeforeMount(async () => {
      await commonStore.getSetting();
      // TODO 根据配置获取
      processing.value = false;
      closeSplashscreen();
    });

    return {
      processing,
      setting,
    };
  },
  render() {
    const { processing, setting } = this;
    if (processing) {
      return "<p>...</p>";
    }
    const isDark = setting.theme === "dark";
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

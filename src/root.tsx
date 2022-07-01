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
import { closeSplashscreen } from "./commands/window";
import { useCommonStore } from "./stores/common";
import App from "./App";
import { storeToRefs } from "pinia";

export default defineComponent({
  name: "Root",
  setup() {
    const commonStore = useCommonStore();
    const { setting } = storeToRefs(commonStore);
    const processing = ref(true);
    onBeforeMount(async () => {
      await commonStore.getSetting();
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

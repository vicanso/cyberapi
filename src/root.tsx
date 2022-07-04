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
import { useAPISettingsStore } from "./stores/api_setting";
import ExLoading from "./components/ExLoading";

const loadingClass = css`
  margin-top: 50px;
`;

export default defineComponent({
  name: "RootView",
  setup() {
    const commonStore = useCommonStore();
    const apiSettingsStore = useAPISettingsStore();
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

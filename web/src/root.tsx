import { defineComponent } from "vue";
import {
  darkTheme,
  NConfigProvider,
  NDialogProvider,
  NGlobalStyle,
  NMessageProvider,
  NNotificationProvider,
  GlobalThemeOverrides,
} from "naive-ui";

import App from "./App";

export default defineComponent({
  name: "RootView",
  render() {
    const themeOverrides: GlobalThemeOverrides = {
      common: {
        bodyColor: "#292040",
      },
    };
    return (
      <NConfigProvider theme={darkTheme} themeOverrides={themeOverrides}>
        <NGlobalStyle />
        <NMessageProvider>
          <NNotificationProvider>
            <NDialogProvider>
              <App />
            </NDialogProvider>
          </NNotificationProvider>
        </NMessageProvider>
      </NConfigProvider>
    );
  },
});

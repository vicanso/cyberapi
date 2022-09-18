import { defineComponent } from "vue";
import {
  darkTheme,
  NConfigProvider,
  NDialogProvider,
  NGlobalStyle,
  NMessageProvider,
  NNotificationProvider,
} from "naive-ui";

import App from "./App";

export default defineComponent({
  name: "RootView",
  render() {
    return (
      <NConfigProvider theme={darkTheme}>
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

import { defineComponent } from "vue";
import { NLayout } from "naive-ui";
import { RouterView } from "vue-router";

import AppHeader from "./components/Header";
import AppFooter from "./components/Footer";

export default defineComponent({
  name: "App",
  render() {
    return (
      <NLayout>
        <AppHeader />
        <RouterView />
        <AppFooter />
      </NLayout>
    );
  },
});

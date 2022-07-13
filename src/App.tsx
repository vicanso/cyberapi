import { defineComponent, onMounted } from "vue";
import {
  NLayout,
  NLayoutContent,
  NLayoutHeader,
  useLoadingBar,
} from "naive-ui";

import "./main.css";
import { setLoadingEvent } from "./router";
import AppHeader from "./views/AppHeader";

export default defineComponent({
  name: "App",
  setup() {
    const loadingBar = useLoadingBar();
    setLoadingEvent(loadingBar.start, loadingBar.finish);
    onMounted(() => {
      loadingBar.finish();
    });
  },
  render() {
    return (
      <NLayout>
        <NLayoutHeader bordered>
          <AppHeader />
        </NLayoutHeader>
        <NLayoutContent>
          <router-view />
        </NLayoutContent>
      </NLayout>
    );
  },
});

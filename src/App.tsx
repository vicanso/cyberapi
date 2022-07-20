import { defineComponent, onMounted } from "vue";
import {
  NLayout,
  NLayoutContent,
  NLayoutHeader,
  useLoadingBar,
  NModal,
} from "naive-ui";

import "./main.css";
import { setLoadingEvent } from "./router";
import AppHeader from "./views/AppHeader";
import { useDialogStore } from "./stores/dialog";
import { storeToRefs } from "pinia";
import AppSetting from "./views/AppSetting";

export default defineComponent({
  name: "App",
  setup() {
    const loadingBar = useLoadingBar();
    const dialogStore = useDialogStore();
    const { showSetting, showCookie } = storeToRefs(dialogStore);
    const closeDialog = () => {
      dialogStore.toggleSettingDialog(false);
      dialogStore.toggleCookieDialog(false);
    };
    setLoadingEvent(loadingBar.start, loadingBar.finish);
    onMounted(() => {
      loadingBar.finish();
    });
    return {
      closeDialog,
      showSetting,
      showCookie,
    };
  },
  render() {
    const { showSetting, showCookie, closeDialog } = this;
    const settingModal = (
      <NModal
        show={showSetting}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <AppSetting />
      </NModal>
    );
    const cookieModal = (
      <NModal
        show={showCookie}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <p>abc</p>
      </NModal>
    );
    return (
      <NLayout>
        {settingModal}
        {cookieModal}
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

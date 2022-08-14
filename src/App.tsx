import { defineComponent, onMounted } from "vue";
import {
  NLayout,
  NLayoutContent,
  NLayoutHeader,
  useLoadingBar,
  NModal,
} from "naive-ui";
import { storeToRefs } from "pinia";

import "./main.css";
import { setLoadingEvent } from "./router";
import AppHeader from "./views/AppHeader";
import { useDialogStore } from "./stores/dialog";
import AppSetting from "./views/AppSetting";
import CookieSetting from "./views/CookieSetting";
import EnvironmentSetting from "./views/EnvironmentSetting";

export default defineComponent({
  name: "App",
  setup() {
    const loadingBar = useLoadingBar();
    const dialogStore = useDialogStore();
    const { showSetting, showCookie, showEnvironment } =
      storeToRefs(dialogStore);
    const closeDialog = () => {
      dialogStore.toggleSettingDialog(false);
      dialogStore.toggleCookieDialog(false);
      dialogStore.toggleEnvironmentDialog(false);
    };
    setLoadingEvent(loadingBar.start, loadingBar.finish);
    onMounted(() => {
      loadingBar.finish();
    });
    return {
      closeDialog,
      showSetting,
      showCookie,
      showEnvironment,
    };
  },
  render() {
    const { showSetting, showCookie, showEnvironment, closeDialog } = this;
    const settingModal = (
      <NModal
        autoFocus={false}
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
        autoFocus={false}
        show={showCookie}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <CookieSetting />
      </NModal>
    );
    const environmentModal = (
      <NModal
        autoFocus={false}
        show={showEnvironment}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <EnvironmentSetting />
      </NModal>
    );
    return (
      <NLayout>
        {settingModal}
        {cookieModal}
        {environmentModal}
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

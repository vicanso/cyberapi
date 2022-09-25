import { defineComponent, onMounted } from "vue";
import { NLayout, NLayoutHeader, useLoadingBar, NModal } from "naive-ui";
import { storeToRefs } from "pinia";

import "./main.css";
import { setLoadingEvent } from "./router";
import AppHeader from "./views/AppHeader";
import { useDialogStore } from "./stores/dialog";
import AppSetting from "./views/AppSetting";
import CookieSetting from "./views/CookieSetting";
import VariableSetting from "./views/VariableSetting";
import StoreSetting from "./views/StoreSetting";
import { VariableCategory } from "./commands/variable";
import { i18nEnvironment, i18nCustomizeVariable } from "./i18n";

export default defineComponent({
  name: "App",
  setup() {
    const loadingBar = useLoadingBar();
    const dialogStore = useDialogStore();
    const {
      showSetting,
      showCookie,
      showEnvironment,
      showStore,
      showCustomizeVariableStore,
    } = storeToRefs(dialogStore);
    const closeDialog = () => {
      dialogStore.toggleSettingDialog(false);
      dialogStore.toggleCookieDialog(false);
      dialogStore.toggleEnvironmentDialog(false);
      dialogStore.toggleStoreDialog(false);
      dialogStore.toggleCustomizeVariableDialog(false);
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
      showStore,
      showCustomizeVariableStore,
    };
  },
  render() {
    const {
      showSetting,
      showCookie,
      showEnvironment,
      showStore,
      showCustomizeVariableStore,
      closeDialog,
    } = this;
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
        <VariableSetting
          category={VariableCategory.Environment}
          title={i18nEnvironment("title")}
          tips={i18nEnvironment("tips")}
        />
      </NModal>
    );
    const customizeVariableModal = (
      <NModal
        autoFocus={false}
        show={showCustomizeVariableStore}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <VariableSetting
          maxWidth={1000}
          category={VariableCategory.Customize}
          title={i18nCustomizeVariable("title")}
          tips={i18nCustomizeVariable("tips")}
        />
      </NModal>
    );
    const storeModal = (
      <NModal
        autoFocus={false}
        show={showStore}
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <StoreSetting />
      </NModal>
    );
    return (
      <NLayout>
        {settingModal}
        {cookieModal}
        {environmentModal}
        {storeModal}
        {customizeVariableModal}
        <NLayoutHeader bordered>
          <AppHeader />
        </NLayoutHeader>
        <div>
          <router-view />
        </div>
      </NLayout>
    );
  },
});

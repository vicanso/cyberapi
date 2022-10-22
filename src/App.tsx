import { defineComponent, onMounted } from "vue";
import { NLayout, NLayoutHeader, useLoadingBar, NModal } from "naive-ui";
import { storeToRefs } from "pinia";

import "./main.css";
import AppHeader from "./views/AppHeader";
import { useDialogStore } from "./stores/dialog";
import AppSetting from "./views/AppSetting";
import CookieSetting from "./views/CookieSetting";
import VariableSetting from "./views/VariableSetting";
import StoreSetting from "./views/StoreSetting";
import { VariableCategory } from "./commands/variable";
import {
  i18nEnvironment,
  i18nCustomizeVariable,
  i18nGlobalReqHeader,
} from "./i18n";

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
      showReqHeader,
    } = storeToRefs(dialogStore);
    const closeDialog = () => {
      dialogStore.$reset();
    };
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
      showReqHeader,
    };
  },
  render() {
    const {
      showSetting,
      showCookie,
      showEnvironment,
      showStore,
      showCustomizeVariableStore,
      showReqHeader,
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
    const reqHeaderModal = (
      <NModal
        autoFocus={false}
        show={showReqHeader}
        closeOnEsc
        onEsc={() => {
          closeDialog();
        }}
        onMaskClick={() => {
          closeDialog();
        }}
      >
        <VariableSetting
          category={VariableCategory.GlobalReqHeaders}
          title={i18nGlobalReqHeader("title")}
          tips={i18nGlobalReqHeader("tips")}
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
        {reqHeaderModal}
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

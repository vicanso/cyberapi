import { defineStore } from "pinia";

export const useDialogStore = defineStore("dialogs", {
  state: () => {
    return {
      showSetting: false,
      showCookie: false,
    };
  },
  actions: {
    toggleSettingDialog(shown: boolean) {
      this.showSetting = shown;
    },
    toggleCookieDialog(shown: boolean) {
      this.showCookie = shown;
    },
  },
});

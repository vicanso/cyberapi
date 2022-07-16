import { defineComponent } from "vue";
import {
  NCard,
  NDescriptions,
  NDescriptionsItem,
  NDivider,
  NP,
  NRadio,
  NRadioGroup,
  NSpace,
  useMessage,
} from "naive-ui";

import { getBodyWidth, showError } from "../helpers/util";
import { i18nSetting } from "../i18n";
import { useSettingStore } from "../stores/setting";
import { storeToRefs } from "pinia";
import { useAppStore } from "../stores/app";

export default defineComponent({
  name: "AppSettingView",
  setup() {
    const settingStore = useSettingStore();
    const appStore = useAppStore();
    const message = useMessage();
    const { theme } = storeToRefs(settingStore);
    // const { version, tauriVersion, platform, arch, os, osVersion } =
    //   storeToRefs(appStore);
    const updateTheme = async (value: string) => {
      try {
        await settingStore.updateTheme(value);
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      theme,
      infos: [
        {
          name: i18nSetting("appVersion"),
          value: appStore.version,
        },
        {
          name: "Tauri",
          value: appStore.tauriVersion,
        },
        {
          name: i18nSetting("platform"),
          value: appStore.platform,
        },
        {
          name: i18nSetting("os"),
          value: appStore.os,
        },
        {
          name: i18nSetting("osVersion"),
          value: appStore.osVersion,
        },
        {
          name: i18nSetting("arch"),
          value: appStore.arch,
        },
      ],

      updateTheme,
    };
  },
  render() {
    const bodyWidth = getBodyWidth();
    const modalWidth = bodyWidth >= 800 ? bodyWidth * 0.6 : bodyWidth - 60;
    const modalStyle = {
      width: `${modalWidth}px`,
    };
    const { theme } = this;
    const descriptionItems = this.infos.map((item) => {
      return (
        <NDescriptionsItem label={item.name} key={item.name}>
          {item.value}
        </NDescriptionsItem>
      );
    });
    return (
      <NCard title={i18nSetting("title")} style={modalStyle}>
        <NP>{i18nSetting("themeTitle")}</NP>
        <NSpace vertical>
          <NRadioGroup value={theme} onUpdateValue={this.updateTheme}>
            <NSpace>
              <NRadio label={i18nSetting("systemTheme")} value="" />
              <NRadio label={i18nSetting("darkTheme")} value="dark" />
              <NRadio label={i18nSetting("lightTheme")} value="light" />
            </NSpace>
          </NRadioGroup>
        </NSpace>
        <NDivider />
        <NP>{i18nSetting("infoTitle")}</NP>
        <NDescriptions>{descriptionItems}</NDescriptions>
      </NCard>
    );
  },
});

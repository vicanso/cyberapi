import { defineComponent, onBeforeUnmount } from "vue";
import {
  NCard,
  NDescriptions,
  NDescriptionsItem,
  NDivider,
  NFormItem,
  NGi,
  NGrid,
  NInputNumber,
  NP,
  NRadio,
  NRadioGroup,
  NSpace,
  useMessage,
} from "naive-ui";

import { getBodyWidth, showError } from "../helpers/util";
import { i18nSetting } from "../i18n";
import { useSettingStore, ResizeType } from "../stores/setting";
import { storeToRefs } from "pinia";
import { useAppStore } from "../stores/app";

export default defineComponent({
  name: "AppSettingView",
  setup() {
    const settingStore = useSettingStore();
    const appStore = useAppStore();
    const message = useMessage();
    const { theme, size, resizeType } = storeToRefs(settingStore);
    const updateTheme = async (value: string) => {
      try {
        await settingStore.updateTheme(value);
      } catch (err) {
        showError(message, err);
      }
    };
    let resized = false;
    onBeforeUnmount(() => {
      if (!resized) {
        return;
      }
      settingStore.resize();
    });
    const updateSize = async (value: number, category: string) => {
      try {
        let { width, height } = settingStore.size;
        if (category == "width") {
          width = value;
        } else {
          height = value;
        }

        await settingStore.updateSize(width, height);
        resized = true;
      } catch (err) {
        showError(message, err);
      }
    };
    const updateResizeType = async (value: string) => {
      try {
        resized = true;
        await settingStore.updateResizeType(value);
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      theme,
      size,
      resizeType,
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
      updateSize,
      updateTheme,
      updateResizeType,
    };
  },
  render() {
    const bodyWidth = getBodyWidth();
    const modalWidth = bodyWidth >= 1000 ? bodyWidth * 0.7 : bodyWidth - 200;
    const modalStyle = {
      width: `${modalWidth}px`,
    };
    const { theme, size, resizeType, updateSize, updateResizeType } = this;
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
        <NP>{i18nSetting("windowSize")}</NP>
        <NGrid xGap={20}>
          <NGi span={8}>
            <NFormItem label={i18nSetting("windowResizeType")}>
              <NRadioGroup value={resizeType} onUpdateValue={updateResizeType}>
                <NSpace>
                  <NRadio
                    label={i18nSetting("windowMaxSize")}
                    value={ResizeType.Max}
                  />
                  <NRadio
                    label={i18nSetting("windowCustomSize")}
                    value={ResizeType.Custom}
                  />
                </NSpace>
              </NRadioGroup>
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nSetting("windowWidth")}>
              <NInputNumber
                class="widthFull"
                placeholder={i18nSetting("windowWidthPlaceholder")}
                min={900}
                disabled={resizeType === ResizeType.Max}
                defaultValue={size?.width || null}
                onUpdateValue={(value) => {
                  updateSize(value || 0, "width");
                }}
              />
            </NFormItem>
          </NGi>
          <NGi span={8}>
            <NFormItem label={i18nSetting("windowHeight")}>
              <NInputNumber
                class="widthFull"
                placeholder={i18nSetting("windowHeightPlaceholder")}
                disabled={resizeType === ResizeType.Max}
                min={600}
                defaultValue={size?.height || null}
                onUpdateValue={(value) => {
                  updateSize(value || 0, "height");
                }}
              />
            </NFormItem>
          </NGi>
        </NGrid>
        <NDivider />
        <NP>{i18nSetting("infoTitle")}</NP>
        <NDescriptions>{descriptionItems}</NDescriptions>
      </NCard>
    );
  },
});

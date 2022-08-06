// API栏目的顶部功能栏
import { defineComponent, inject, onBeforeUnmount, PropType } from "vue";
import { css } from "@linaria/core";
import { NDropdown, NButton, NGi, NGrid, NInput, NIcon } from "naive-ui";

import { i18nCollection, i18nCommon } from "../../i18n";
import { SettingType } from "../../stores/api_setting";
import { AnalyticsOutline, FolderOpenOutline } from "@vicons/ionicons5";
import {
  hotKeyCreateFolder,
  hotKeyCreateHTTPSetting,
  hotKeyMatchCreateHTTPSetting,
} from "../../helpers/hot_key";
import {
  addFolderDefaultValue,
  addFolderKey,
  addHTTPSettingDefaultValue,
  addHTTPSettingKey,
} from "../../constants/provide";

const addDropdownClass = css`
  .label {
    min-width: 180px;
  }
  .hotKey {
    float: right;
  }
`;

export default defineComponent({
  name: "APISettingTreeHeader",
  props: {
    onFilter: {
      type: Function as PropType<(value: string) => void>,
      required: true,
    },
  },
  setup() {
    const addHTTPSetting = inject(
      addHTTPSettingKey,
      addHTTPSettingDefaultValue
    );
    const addFolder = inject(addFolderKey, addFolderDefaultValue);

    const handleKeydown = (e: KeyboardEvent) => {
      if (hotKeyMatchCreateHTTPSetting(e)) {
        addHTTPSetting("");
        return;
      }
    };
    document.addEventListener("keydown", handleKeydown);
    onBeforeUnmount(() => {
      document.removeEventListener("keydown", handleKeydown);
    });

    return {
      addHTTPSetting,
      addFolder,
      text: {
        add: i18nCommon("add"),
        placeholder: i18nCollection("filterPlaceholder"),
      },
    };
  },
  render() {
    const options = [
      {
        label: `${i18nCollection(
          "newHTTPRequest"
        )} | ${hotKeyCreateHTTPSetting()}`,
        key: SettingType.HTTP,
        icon: () => (
          <NIcon>
            <AnalyticsOutline />
          </NIcon>
        ),
      },
      {
        label: `${i18nCollection("newFolder")} | ${hotKeyCreateFolder()}`,
        key: SettingType.Folder,
        icon: () => (
          <NIcon>
            <FolderOpenOutline />
          </NIcon>
        ),
      },
    ];
    const { text } = this;
    return (
      <NGrid xGap={12}>
        <NGi span={16}>
          <NInput
            type="text"
            clearable
            placeholder={text.placeholder}
            onInput={(value: string) => {
              this.$props.onFilter(value.toLowerCase());
            }}
          />
        </NGi>
        <NGi span={8}>
          <NDropdown
            class={addDropdownClass}
            trigger="click"
            options={options}
            renderLabel={(option) => {
              const arr = (option.label as string).split(" | ");
              const hotkey =
                arr.length === 2 ? (
                  <span class="hotKey">{arr[1]}</span>
                ) : undefined;

              return (
                <div class="label">
                  {arr[0]}
                  {hotkey}
                </div>
              );
            }}
            onSelect={(key: string) => {
              switch (key) {
                case SettingType.HTTP:
                  this.addHTTPSetting("");
                  break;
                case SettingType.Folder:
                  this.addFolder();
                  break;
              }
            }}
          >
            <NButton class="widthFull">{text.add}</NButton>
          </NDropdown>
        </NGi>
      </NGrid>
    );
  },
});

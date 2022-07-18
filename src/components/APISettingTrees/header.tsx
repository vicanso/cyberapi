import { defineComponent, PropType } from "vue";
import { NDropdown, NButton, NGi, NGrid, NInput } from "naive-ui";

import { i18nApiSetting, i18nCommon } from "../../i18n";
import { SettingType } from "../../stores/api_setting";

export default defineComponent({
  name: "APISettingTreesHeader",
  props: {
    onFilter: {
      type: Function as PropType<(value: string) => void>,
      required: true,
    },
  },
  setup() {
    return {
      text: {
        add: i18nCommon("add"),
        placeholder: i18nApiSetting("filterPlaceholder"),
      },
      options: [
        {
          label: i18nApiSetting("newHTTPRequest"),
          key: SettingType.HTTP,
        },
        {
          label: i18nApiSetting("newFolder"),
          key: SettingType.Folder,
        },
      ],
    };
  },
  render() {
    const { text, options } = this;
    return (
      <NGrid xGap={12}>
        <NGi span={16}>
          <NInput
            type="text"
            clearable
            placeholder={text.placeholder}
            onChange={(value: string) => {
              this.$props.onFilter(value);
            }}
          />
        </NGi>
        <NGi span={8}>
          <NDropdown
            trigger="click"
            options={options}
            onSelect={(key: string) => {
              switch (key) {
                case SettingType.HTTP:
                  // this.addAPI(key, "");
                  break;
                case SettingType.Folder:
                  // this.addFolder();
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

import { defineComponent, PropType } from "vue";
import {
  NDropdown,
  NButton,
  NGi,
  NGrid,
  NInput,
  useDialog,
  useMessage,
} from "naive-ui";
import { useRoute } from "vue-router";

import { i18nCollection, i18nCommon } from "../../i18n";
import { SettingType, useAPISettingStore } from "../../stores/api_setting";
import { ExFormItem } from "../ExForm";
import ExDialog from "../ExDialog";
import { newDefaultAPIFolder } from "../../commands/api_folder";
import { useAPIFolderStore } from "../../stores/api_folder";
import { showError } from "../../helpers/util";
import { newDefaultAPISetting } from "../../commands/api_setting";

const getFolderFormItems = (): ExFormItem[] => {
  return [
    {
      key: "name",
      label: i18nCommon("name"),
      placeholer: i18nCommon("namePlaceholder"),
      rule: {
        required: true,
        message: i18nCommon("nameRequireError"),
        trigger: "blur",
      },
    },
  ];
};

const getSettingFormItems = (): ExFormItem[] => {
  return [
    {
      key: "name",
      label: i18nCommon("name"),
      placeholer: i18nCommon("namePlaceholder"),
      rule: {
        required: true,
        message: i18nCommon("nameRequireError"),
        trigger: "blur",
      },
    },
  ];
};

export default defineComponent({
  name: "APISettingTreeHeader",
  props: {
    onFilter: {
      type: Function as PropType<(value: string) => void>,
      required: true,
    },
  },
  setup() {
    const folderStore = useAPIFolderStore();
    const settingStore = useAPISettingStore();
    const dialog = useDialog();
    const message = useMessage();
    const route = useRoute();
    const collection = route.query.id as string;
    const addFolder = () => {
      ExDialog({
        dialog,
        title: i18nCollection("newFolder"),
        formItems: getFolderFormItems(),
        onConfirm: async (data) => {
          const folder = newDefaultAPIFolder();
          folder.collection = collection;
          folder.name = data.name as string;
          try {
            await folderStore.add(folder);
          } catch (err) {
            showError(message, err);
          }
        },
      });
    };
    const addHTTPSetting = () => {
      ExDialog({
        dialog,
        title: i18nCollection("newHTTPRequest"),
        formItems: getSettingFormItems(),
        onConfirm: async (data) => {
          const setting = newDefaultAPISetting();
          setting.category = SettingType.HTTP;
          setting.collection = collection;
          setting.name = data.name as string;
          try {
            await settingStore.add(setting);
          } catch (err) {
            showError(message, err);
          }
        },
      });
    };
    return {
      addHTTPSetting,
      addFolder,
      text: {
        add: i18nCommon("add"),
        placeholder: i18nCollection("filterPlaceholder"),
      },
      options: [
        {
          label: i18nCollection("newHTTPRequest"),
          key: SettingType.HTTP,
        },
        {
          label: i18nCollection("newFolder"),
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
                  this.addHTTPSetting();
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

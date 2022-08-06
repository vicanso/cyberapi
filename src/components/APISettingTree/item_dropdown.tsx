// API功能下拉选项框
import {
  AddOutline,
  ChevronDownOutline,
  CreateOutline,
  TrashOutline,
} from "@vicons/ionicons5";
import { NDropdown, NIcon, useDialog, useMessage } from "naive-ui";
import { css } from "@linaria/core";
import { defineComponent, inject } from "vue";
import { showError } from "../../helpers/util";
import { i18nCollection, i18nCommon } from "../../i18n";
import { useAPIFolderStore } from "../../stores/api_folder";
import { SettingType, useAPISettingStore } from "../../stores/api_setting";
import { HandleKey } from "../../constants/handle_key";
import ExDialog from "../ExDialog";
import {
  addHTTPSettingDefaultValue,
  addHTTPSettingKey,
} from "../../constants/provide";

const dropdonwClass = css`
  .option {
    margin: 0 10px 0 0;
  }
`;

export default defineComponent({
  name: "APISettingTreeItemDropdown",
  props: {
    id: {
      type: String,
      required: true,
    },
    apiSettingType: {
      type: String,
      required: true,
    },
  },
  setup(props) {
    const dialog = useDialog();
    const message = useMessage();
    const settingStore = useAPISettingStore();
    const folderStore = useAPIFolderStore();

    const addHTTPSetting = inject(
      addHTTPSettingKey,
      addHTTPSettingDefaultValue
    );
    const handleSelect = (key: string) => {
      const { id, apiSettingType } = props;
      let name = "";
      let isFolder = false;
      if (apiSettingType === SettingType.Folder) {
        isFolder = true;
        name = folderStore.findByID(id).name;
      } else {
        name = settingStore.findByID(id).name;
      }

      switch (key) {
        case HandleKey.Delete:
          {
            const content = i18nCollection("deleteSettingContent").replace(
              "%s",
              name
            );
            const d = dialog.warning({
              title: i18nCollection("deleteSetting"),
              content: content,
              positiveText: i18nCommon("confirm"),
              onPositiveClick: async () => {
                d.loading = true;
                try {
                  if (isFolder) {
                    await folderStore.remove(id);
                  } else {
                    await settingStore.remove(id);
                  }
                } catch (err) {
                  showError(message, err);
                }
              },
            });
          }
          break;
        case HandleKey.Modify:
          {
            ExDialog({
              dialog,
              title: i18nCollection("modifySetting"),
              formItems: [
                {
                  key: "name",
                  label: i18nCommon("name"),
                  defaultValue: name,
                  placeholer: i18nCommon("namePlaceholder"),
                  rule: {
                    required: true,
                    message: i18nCommon("nameRequireError"),
                    trigger: "blur",
                  },
                },
              ],
              onConfirm: async (data) => {
                try {
                  if (isFolder) {
                    await folderStore.updateByID(id, data);
                  } else {
                    await settingStore.updateByID(id, data);
                  }
                } catch (err) {
                  showError(message, err);
                }
              },
            });
          }
          break;
        case HandleKey.Create:
          {
            addHTTPSetting(id);
          }
          break;
        default:
          break;
      }
    };
    return {
      handleSelect,
    };
  },
  render() {
    const { apiSettingType } = this.$props;
    const options = [
      {
        label: i18nCommon("modify"),
        key: HandleKey.Modify,
        icon: () => (
          <NIcon>
            <CreateOutline />
          </NIcon>
        ),
      },
      {
        label: i18nCommon("delete"),
        key: HandleKey.Delete,
        icon: () => (
          <NIcon>
            <TrashOutline />
          </NIcon>
        ),
      },
    ];
    if (apiSettingType === SettingType.Folder) {
      options.unshift({
        label: i18nCollection("newHTTPRequest"),
        key: HandleKey.Create,
        icon: () => (
          <NIcon>
            <AddOutline />
          </NIcon>
        ),
      });
    }
    return (
      <NDropdown
        class={dropdonwClass}
        options={options}
        trigger="click"
        onSelect={this.handleSelect}
        renderLabel={(option) => {
          return <span class="option">{option.label}</span>;
        }}
      >
        <span class="preventDefault itemDropitem">
          <NIcon class="preventDefault">
            <ChevronDownOutline class="preventDefault" />
          </NIcon>
        </span>
      </NDropdown>
    );
  },
});

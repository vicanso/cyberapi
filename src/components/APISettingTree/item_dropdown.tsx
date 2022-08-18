// API功能下拉选项框
import {
  AddOutline,
  ChevronDownOutline,
  CreateOutline,
  LinkOutline,
  LogOutOutline,
  TrashOutline,
} from "@vicons/ionicons5";
import { NDropdown, NIcon, useDialog, useMessage } from "naive-ui";
import { defineComponent, inject } from "vue";
import { useRoute } from "vue-router";
import { writeText } from "@tauri-apps/api/clipboard";

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
import { usePinRequestStore } from "../../stores/pin_request";
import { convertRequestToCURL } from "../../commands/http_request";

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
    const pinRequestStore = usePinRequestStore();
    const apiSettingStore = useAPISettingStore();
    const route = useRoute();
    const collection = route.query.id as string;

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
        case HandleKey.ExportCURL:
          {
            const req = apiSettingStore.getHTTPRequestFillENV(id);
            const curl = convertRequestToCURL(req);
            writeText(curl)
              .then(() => {
                message.success(i18nCollection("exportCURLSuccess"));
              })
              .catch((err) => {
                showError(message, err);
              });
          }
          break;
        case HandleKey.Pin:
          {
            pinRequestStore.add(collection, {
              id,
            });
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
        label: i18nCollection("modifySetting"),
        key: HandleKey.Modify,
        icon: () => (
          <NIcon>
            <CreateOutline />
          </NIcon>
        ),
      },
      {
        label: i18nCollection("deleteSetting"),
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
    } else {
      // TODO 添加从curl导出
      options.push(
        {
          label: i18nCollection("exportCURL"),
          key: HandleKey.ExportCURL,
          icon: () => (
            <NIcon>
              <LogOutOutline />
            </NIcon>
          ),
        },
        {
          label: i18nCollection("pinRequest"),
          key: HandleKey.Pin,
          icon: () => (
            <NIcon>
              <LinkOutline />
            </NIcon>
          ),
        }
      );
    }
    return (
      <NDropdown
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

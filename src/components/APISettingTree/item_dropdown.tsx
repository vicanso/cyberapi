// API功能下拉选项框
import {
  AddOutline,
  ChevronDownOutline,
  CopyOutline,
  CreateOutline,
  DownloadOutline,
  FolderOpenOutline,
  LinkOutline,
  PinOutline,
  TrashOutline,
} from "@vicons/ionicons5";
import { NDropdown, NIcon, useDialog, useMessage } from "naive-ui";
import { defineComponent, inject } from "vue";
import { useRoute } from "vue-router";
import { css } from "@linaria/core";
import { DropdownMixedOption } from "naive-ui/es/dropdown/src/interface";
import { BaseDirectory, writeTextFile } from "@tauri-apps/api/fs";

import {
  readTextFromClipboard,
  showError,
  writeTextToClipboard,
} from "../../helpers/util";
import { i18nCollection, i18nCommon } from "../../i18n";
import { useAPIFolderStore } from "../../stores/api_folder";
import { SettingType, useAPISettingStore } from "../../stores/api_setting";
import { HandleKey } from "../../constants/handle_key";
import ExDialog, { newImportDialog } from "../ExDialog";
import {
  addFolderDefaultValue,
  addFolderKey,
  addHTTPSettingDefaultValue,
  addHTTPSettingKey,
} from "../../constants/provide";
import { usePinRequestStore } from "../../stores/pin_request";
import { convertRequestToCURL } from "../../commands/http_request";
import { APISetting } from "../../commands/api_setting";
import { APIFolder } from "../../commands/api_folder";

const dropdownClass = css`
  .n-dropdown-option {
    margin: 2px 0;
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
    const apiFolderStore = useAPIFolderStore();
    const pinRequestStore = usePinRequestStore();
    const apiSettingStore = useAPISettingStore();
    const route = useRoute();
    const collection = route.query.id as string;

    const addHTTPSetting = inject(
      addHTTPSettingKey,
      addHTTPSettingDefaultValue
    );
    const addFolder = inject(addFolderKey, addFolderDefaultValue);

    const hanldeImport = async (id: string) => {
      try {
        const data = (await readTextFromClipboard()) || "";
        newImportDialog({
          dialog,
          collection,
          folder: id,
          data,
        });
      } catch (err) {
        showError(message, err);
      }
    };
    const handleExport = async (id: string) => {
      try {
        const apiSettings: APISetting[] = [];
        const folders: APIFolder[] = [];
        const appendChildren = (folderId: string) => {
          const folder = apiFolderStore.findByID(folderId);
          if (!folder) {
            return;
          }
          folders.push(folder);
          folder.children.split(",").forEach((child) => {
            const apiSetting = apiSettingStore.findByID(child);
            if (apiSetting) {
              apiSettings.push(apiSetting);
            } else {
              // folder
              appendChildren(child);
            }
          });
        };
        appendChildren(id);
        const arr: unknown[] = [];
        folders.forEach((folder) => arr.push(folder));
        apiSettings.forEach((apiSetting) => arr.push(apiSetting));
        const data = JSON.stringify(arr, null, 2);
        const file = `cyberapi-${new Date().toISOString()}.json`;
        await writeTextFile(file, data, {
          dir: BaseDirectory.Download,
        });
        message.info(i18nCollection("exportSettingsSuccess"));
      } catch (err) {
        showError(message, err);
      }
    };
    const handleSelect = (key: string) => {
      const { id, apiSettingType } = props;
      let name = "";
      let isFolder = false;
      if (apiSettingType === SettingType.Folder) {
        isFolder = true;
        name = apiFolderStore.findByID(id).name;
      } else {
        name = apiSettingStore.findByID(id).name;
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
                    await apiFolderStore.remove(id);
                  } else {
                    await apiSettingStore.remove(id);
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
                    await apiFolderStore.updateByID(id, data);
                  } else {
                    await apiSettingStore.updateByID(id, data);
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
        case HandleKey.CreateFolder:
          {
            addFolder(id);
          }
          break;
        case HandleKey.CopyAsCURL:
          {
            const req = apiSettingStore.getHTTPRequestFillENV(id);
            convertRequestToCURL(req)
              .then(writeTextToClipboard)
              .then(() => {
                message.success(i18nCollection("copyAsCURLSuccess"));
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
        case HandleKey.ImportSettings:
          {
            hanldeImport(id);
          }
          break;
        case HandleKey.ExportSettings:
          {
            handleExport(id);
          }
          break;
        case HandleKey.Copy:
          {
            const setting = apiSettingStore.findByID(id);
            writeTextToClipboard(JSON.stringify(setting, null, 2))
              .then(() => {
                message.success(i18nCollection("copySettingSuccess"));
              })
              .catch((err) => {
                showError(message, err);
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
    const options: DropdownMixedOption[] = [
      {
        label: i18nCollection("modifySetting"),
        key: HandleKey.Modify,
        icon: () => (
          <NIcon>
            <CreateOutline />
          </NIcon>
        ),
      },
    ];
    if (apiSettingType === SettingType.Folder) {
      options.unshift(
        {
          label: i18nCollection("newHTTPRequest"),
          key: HandleKey.Create,
          icon: () => (
            <NIcon>
              <AddOutline />
            </NIcon>
          ),
        },
        {
          label: i18nCollection("newFolder"),
          key: HandleKey.CreateFolder,
          icon: () => (
            <NIcon>
              <FolderOpenOutline />
            </NIcon>
          ),
        }
      );
      options.push(
        {
          label: i18nCollection("exportSettings"),
          key: HandleKey.ExportSettings,
          icon: () => (
            <NIcon>
              <DownloadOutline class="rotate90" />
            </NIcon>
          ),
        },
        {
          label: i18nCollection("importSettings"),
          key: HandleKey.ImportSettings,
          icon: () => (
            <NIcon>
              <DownloadOutline class="rotate270" />
            </NIcon>
          ),
        }
      );
    } else {
      options.push(
        {
          label: i18nCollection("copyAsCURL"),
          key: HandleKey.CopyAsCURL,
          icon: () => (
            <NIcon>
              <LinkOutline />
            </NIcon>
          ),
        },

        {
          label: i18nCollection("pinRequest"),
          key: HandleKey.Pin,
          icon: () => (
            <NIcon>
              <PinOutline />
            </NIcon>
          ),
        },
        {
          label: i18nCollection("copySetting"),
          key: HandleKey.Copy,
          icon: () => (
            <NIcon>
              <CopyOutline />
            </NIcon>
          ),
        }
      );
    }
    options.push(
      {
        type: "divider",
        key: HandleKey.Divider,
      },
      {
        label: i18nCollection("deleteSetting"),
        key: HandleKey.Delete,
        icon: () => (
          <NIcon>
            <TrashOutline />
          </NIcon>
        ),
      }
    );
    return (
      <NDropdown
        class={dropdownClass}
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

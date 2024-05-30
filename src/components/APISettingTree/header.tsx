// API栏目的顶部功能栏
import { defineComponent, inject, onBeforeUnmount, PropType } from "vue";
import { css } from "@linaria/core";
import {
  NDropdown,
  NButton,
  NGi,
  NGrid,
  NInput,
  NIcon,
  useMessage,
  useDialog,
} from "naive-ui";
import { DropdownMixedOption } from "naive-ui/es/dropdown/src/interface";

import { i18nCollection, i18nCommon } from "../../i18n";
import { SettingType, useAPISettingStore } from "../../stores/api_setting";
import {
  AnalyticsOutline,
  DownloadOutline,
  FolderOpenOutline,
} from "@vicons/ionicons5";
import {
  hotKeyCreateFolder,
  hotKeyCreateHTTPSetting,
  hotKeyMatchCreateFolder,
  hotKeyMatchCreateHTTPSetting,
} from "../../helpers/hot_key";
import {
  addFolderDefaultValue,
  addFolderKey,
  addHTTPSettingDefaultValue,
  addHTTPSettingKey,
} from "../../constants/provide";
import {
  readTextFromClipboard,
  showError,
  writeSettingToDownload,
} from "../../helpers/util";
import { useRoute } from "vue-router";
import { useAPIFolderStore } from "../../stores/api_folder";
import { useAPICollectionStore } from "../../stores/api_collection";
import { HandleKey } from "../../constants/handle_key";
import { newImportDialog } from "../ExDialog";

const collapseWidth = 50;

const headerClass = css`
  margin-right: ${collapseWidth}px;
  position: relative;
  .collapse {
    position: absolute;
    top: 0;
    right: ${-collapseWidth}px;
    bottom: 0;
    width: ${collapseWidth}px;
    .n-button {
      margin-left: 10px;
    }
  }
`;

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
    const route = useRoute();
    const message = useMessage();
    const dialog = useDialog();
    const apiFolderStore = useAPIFolderStore();
    const apiSettingStore = useAPISettingStore();
    const collectionStore = useAPICollectionStore();

    const collection = route.query.collection as string;
    const addHTTPSetting = inject(
      addHTTPSettingKey,
      addHTTPSettingDefaultValue,
    );
    const addFolder = inject(addFolderKey, addFolderDefaultValue);

    const handleKeydown = (e: KeyboardEvent) => {
      if (hotKeyMatchCreateFolder(e)) {
        addFolder("");
        return;
      }
      if (hotKeyMatchCreateHTTPSetting(e)) {
        addHTTPSetting("");
        return;
      }
    };
    document.addEventListener("keydown", handleKeydown);
    onBeforeUnmount(() => {
      document.removeEventListener("keydown", handleKeydown);
    });

    const handleCloseAllFolders = async () => {
      try {
        await collectionStore.closeAllFolders(collection);
      } catch (err) {
        showError(message, err);
      }
    };

    const handleImport = async () => {
      let data = "";
      try {
        data = (await readTextFromClipboard()) || "";
      } catch (err) {
        showError(message, err);
      } finally {
        newImportDialog({
          dialog,
          collection,
          data,
        });
      }
    };

    const handleExport = async () => {
      const arr: unknown[] = [];
      apiFolderStore.apiFolders.forEach((folder) => arr.push(folder));
      apiSettingStore.apiSettings.forEach((apiSetting) => arr.push(apiSetting));
      try {
        let name = "unknown";
        const result = collectionStore.findByID(collection);
        if (result) {
          name = result.name;
        }
        await writeSettingToDownload(arr, name);
        message.info(i18nCollection("exportSettingsSuccess"));
      } catch (err) {
        showError(message, err);
      }
    };

    return {
      handleImport,
      handleExport,
      addHTTPSetting,
      addFolder,
      handleCloseAllFolders,
      text: {
        add: i18nCommon("add"),
        placeholder: i18nCollection("filterPlaceholder"),
      },
    };
  },
  render() {
    const options: DropdownMixedOption[] = [
      {
        label: `${i18nCollection(
          "newHTTPRequest",
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
      {
        type: "divider",
        key: "divider",
      },
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
      },
    ];
    const { text } = this;
    const inputProps = {
      spellcheck: false,
    };
    return (
      <div class={headerClass}>
        <NGrid xGap={8}>
          <NGi span={16}>
            <NInput
              type="text"
              clearable
              inputProps={inputProps}
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
                    this.addFolder("");
                    break;
                  case HandleKey.ImportSettings:
                    this.handleImport();
                    break;
                  case HandleKey.ExportSettings:
                    this.handleExport();
                    break;
                }
              }}
            >
              <NButton class="widthFull">{text.add}</NButton>
            </NDropdown>
          </NGi>
        </NGrid>
        <div class="collapse">
          <NButton
            onClick={() => {
              this.handleCloseAllFolders();
            }}
          >
            <NIcon size={18}>
              <svg
                viewBox="0 0 24 24"
                fill="none"
                xmlns="http://www.w3.org/2000/svg"
              >
                <path
                  d="M22 18h-6M15 7l-1.8-2.4c-.589-.785-.883-1.178-1.306-1.389C11.472 3 10.981 3 10 3H7c-1.886 0-2.828 0-3.414.586C3 4.172 3 5.114 3 7v10c0 1.886 0 2.828.586 3.414C4.172 21 5.114 21 7 21h4m4-14h2c1.886 0 2.828 0 3.414.586C21 8.172 21 9.114 21 11v.5M15 7H3"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                />
              </svg>
            </NIcon>
          </NButton>
        </div>
      </div>
    );
  },
});

// API应用配置列表
import { css } from "@linaria/core";
import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount } from "vue";
import {
  useMessage,
  NDropdown,
  NButton,
  NGi,
  NGrid,
  NInput,
  useDialog,
  NForm,
  NFormItem,
  NTree,
  NIcon,
  TreeDropInfo,
} from "naive-ui";
import { FolderOutline } from "@vicons/ionicons5";

import { useAPISettingsStore, SettingType } from "../../stores/api_setting";
import { showError } from "../../helpers/util";
import { i18nCollection, i18nCommon } from "../../i18n";
import ExLoading from "../ExLoading";
import SuffixDropdown from "./suffix_dropdown";

const searchBarClass = css`
  padding: 10px;
`;

const apiSettingClass = css`
  .n-tree-node {
    height: 35px;
  }
  .n-tree-node-switcher {
    display: none;
  }
  .n-button {
    display: none;
  }
  .n-tree-node:hover .n-button {
    display: inline-flex;
  }
`;

const loadingClass = css`
  margin-top: 50px;
`;

export default defineComponent({
  name: "APISettingTrees",
  setup() {
    const apiSettingsStore = useAPISettingsStore();

    const { apiSettingTrees, listProcessing } = storeToRefs(apiSettingsStore);
    const message = useMessage();
    const dialog = useDialog();

    onBeforeMount(async () => {
      try {
        await apiSettingsStore.list();
      } catch (err) {
        showError(message, err);
      }
    });

    // 添加api接口
    const addAPI = async (key: string, folder: string) => {
      try {
        await apiSettingsStore.add(folder);
        await apiSettingsStore.list();
      } catch (err) {
        showError(message, err);
      }
    };
    // 创建api目录
    const addFolder = async () => {
      let name = "";
      const dom = (
        <NForm>
          <NFormItem label={i18nCommon("name")} path="name">
            <NInput
              clearable
              placeholder={i18nCommon("namePlaceholder")}
              onInput={(value: string) => {
                name = value;
              }}
            />
          </NFormItem>
        </NForm>
      );
      // 弹窗确认是否创建
      const d = dialog.info({
        title: i18nCollection("newFolder"),
        content: () => dom,
        positiveText: i18nCommon("confirm"),
        onPositiveClick: () => {
          if (!name) {
            showError(message, new Error(i18nCommon("nameRequireError")));
            return Promise.reject();
          }
          d.loading = true;
          return new Promise((resolve, reject) => {
            apiSettingsStore
              .addFolder(name)
              .then(() => {
                return apiSettingsStore.list();
              })
              .then(resolve)
              .catch(reject);
          });
        },
      });
    };

    const isFolder = (id: string) => {
      return !apiSettingsStore.apiSettingMap.has(id);
    };

    const handleDrop = async ({
      node,
      dragNode,
      dropPosition,
    }: TreeDropInfo) => {
      const isDragToFolder = isFolder(node.key as string);
      switch (dropPosition) {
        case "inside":
          // 如果inside非fold，则不处理
          if (!isDragToFolder) {
            return;
          }
          await apiSettingsStore.addToFolder(
            dragNode.key as string,
            node.key as string
          );
          await apiSettingsStore.list();
          break;
        default:
          break;
      }
    };

    return {
      addAPI,
      addFolder,
      isFolder,
      handleDrop,
      text: {
        add: i18nCommon("add"),
        placeholder: i18nCollection("filterPlaceholder"),
      },
      apiSettingTrees,
      listProcessing,
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
    const { text, options, apiSettingTrees, listProcessing } = this;

    let settings = <ExLoading class={loadingClass} />;
    if (!listProcessing) {
      settings = (
        <NTree
          class={apiSettingClass}
          expand-on-click={true}
          block-line
          draggable
          renderPrefix={(option) => {
            if (this.isFolder(option.option.key as string)) {
              return (
                <NIcon>
                  <FolderOutline />
                </NIcon>
              );
            }
          }}
          renderSuffix={(option) => {
            return (
              <SuffixDropdown
                id={option.option.key as string}
                onSelect={(category) => {
                  console.dir(category);
                }}
              />
            );
          }}
          onDrop={this.handleDrop}
          data={apiSettingTrees}
        />
      );
    }

    return (
      <div class={searchBarClass}>
        <NGrid xGap={12}>
          <NGi span={16}>
            <NInput
              type="text"
              clearable
              placeholder={text.placeholder}
              onChange={(value: string) => {
                console.dir(value);
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
                    this.addAPI(key, "");
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
        {settings}
      </div>
    );
  },
});

// API应用配置列表
import { css } from "@linaria/core";
import { defineComponent, provide, ref } from "vue";
import { useDialog, useMessage } from "naive-ui";

import { SettingType, useAPISettingStore } from "../../stores/api_setting";
import APISettingTreeHeader from "./header";
import APISettingTreeItems from "./items";
import ExDialog from "../ExDialog";
import { i18nCollection, i18nCommon } from "../../i18n";
import { ExFormItem } from "../ExForm";
import { newDefaultAPISetting } from "../../commands/api_setting";
import { useRoute } from "vue-router";
import { showError } from "../../helpers/util";
import { useAPIFolderStore } from "../../stores/api_folder";
import { newDefaultAPIFolder } from "../../commands/api_folder";
import { addFolderKey, addHTTPSettingKey } from "../../constants/provide";

const treesClass = css`
  padding: 10px;
`;

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

export default defineComponent({
  name: "APISettingTree",
  setup() {
    const keyword = ref("");
    const settingStore = useAPISettingStore();
    const folderStore = useAPIFolderStore();
    const dialog = useDialog();
    const route = useRoute();
    const message = useMessage();
    const collection = route.query.id as string;

    provide(addHTTPSettingKey, (folder: string) => {
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
            if (folder) {
              await folderStore.addChild({
                id: folder,
                child: setting.id,
                index: 0,
              });
            }
          } catch (err) {
            showError(message, err);
          }
        },
      });
    });
    provide(addFolderKey, () => {
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
    });
    return {
      keyword,
    };
  },
  render() {
    return (
      <div class={treesClass}>
        <APISettingTreeHeader
          onFilter={(value: string) => {
            this.keyword = value;
          }}
        />
        <APISettingTreeItems keyword={this.keyword} />
      </div>
    );
  },
});

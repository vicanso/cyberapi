// API功能下拉选项框
import { ChevronDownOutline, TrashOutline } from "@vicons/ionicons5";
import { NDropdown, NIcon, useDialog, useMessage } from "naive-ui";
import { defineComponent } from "vue";
import { showError } from "../../helpers/util";
import { i18nCollection, i18nCommon } from "../../i18n";
import { useAPIFolderStore } from "../../stores/api_folder";
import { SettingType, useAPISettingStore } from "../../stores/api_setting";

const deleteKey = "delete";

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
    const handleSelect = (key: string) => {
      const { id, apiSettingType } = props;
      switch (key) {
        case deleteKey:
          {
            const d = dialog.warning({
              title: i18nCollection("deleteSetting"),
              content: i18nCollection("deleteSettingContent"),
              positiveText: i18nCommon("confirm"),
              onPositiveClick: async () => {
                d.loading = true;
                try {
                  if (apiSettingType === SettingType.HTTP) {
                    await settingStore.remove(id);
                  } else {
                    await folderStore.remove(id);
                  }
                } catch (err) {
                  showError(message, err);
                }
              },
            });
          }
          break;
        default:
          break;
      }
      console.dir(key);
    };
    return {
      handleSelect,
    };
  },
  render() {
    const options = [
      {
        label: i18nCommon("delete"),
        key: "delete",
        icon: () => (
          <NIcon>
            <TrashOutline />
          </NIcon>
        ),
      },
    ];
    return (
      <NDropdown options={options} trigger="click" onSelect={this.handleSelect}>
        <span class="preventDefault itemDropitem">
          <NIcon class="preventDefault">
            <ChevronDownOutline class="preventDefault" />
          </NIcon>
        </span>
      </NDropdown>
    );
  },
});

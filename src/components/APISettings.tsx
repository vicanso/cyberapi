import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount } from "vue";
import { useMessage, NDropdown, NButton, NGi, NGrid } from "naive-ui";

import { useAPISettingsStore } from "../stores/api_setting";
import { showError } from "../helpers/util";

export default defineComponent({
  name: "APISettings",
  setup() {
    const apiSettingsStore = useAPISettingsStore();

    const { apiSettings, processing } = storeToRefs(apiSettingsStore);
    const message = useMessage();

    onBeforeMount(async () => {
      try {
        await apiSettingsStore.list();
      } catch (err) {
        showError(message, err);
      }
    });
    return {
      apiSettings,
      processing,
      options: [
        {
          label: "HTTP Request",
          key: "http",
        },
        {
          label: "New Folder",
          key: "folder",
        },
      ],
    };
  },
  render() {
    const { apiSettings, options } = this;
    console.dir(apiSettings);
    return (
      <div>
        <NGrid>
          <NGi span={16}></NGi>
          <NGi span={8}>
            <NDropdown
              trigger="click"
              options={options}
              onSelect={(key: string) => {
                console.dir(key);
              }}
            >
              <NButton class="widthFull">Add</NButton>
            </NDropdown>
          </NGi>
        </NGrid>
      </div>
    );
  },
});

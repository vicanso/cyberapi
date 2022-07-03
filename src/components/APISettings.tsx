import { css } from "@linaria/core";
import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount } from "vue";
import { useMessage, NDropdown, NButton, NGi, NGrid, NInput } from "naive-ui";

import { useAPISettingsStore } from "../stores/api_setting";
import { showError } from "../helpers/util";
import { i18nAppSetting } from "../i18n";

const searchBarClass = css`
  padding: 10px;
`;

enum CreateType {
  HTTP = "http",
  Folder = "folder",
}

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

    const addHandler = async (key: string, folder: string) => {
      try {
        switch (key) {
          case CreateType.HTTP:
            await apiSettingsStore.add(folder);
            await apiSettingsStore.list();
            break;
        }
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      addHandler,
      text: {
        add: i18nAppSetting("add"),
        placeholder: i18nAppSetting("filterPlaceholder"),
      },
      apiSettings,
      processing,
      options: [
        {
          label: i18nAppSetting("newHTTPRequest"),
          key: CreateType.HTTP,
        },
        {
          label: i18nAppSetting("newFolder"),
          key: CreateType.Folder,
        },
      ],
    };
  },
  render() {
    const { text, options, apiSettings, processing, addHandler } = this;

    let settings = <p>...</p>;
    if (!processing) {
      const list = apiSettings.map((item) => {
        let name = item.name;
        if (!name) {
          name = i18nAppSetting("defaultName");
        }
        return <li key={item.id}>{name}</li>;
      });
      if (list.length) {
        settings = <ul>{list}</ul>;
      } else {
        settings = <p>please add first</p>;
      }
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
                addHandler(key, "");
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

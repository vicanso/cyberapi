import { defineComponent, watch, ref, onBeforeUnmount } from "vue";
import { css } from "@linaria/core";
import {
  NDivider,
  NDropdown,
  NIcon,
  NInput,
  NSelect,
  NTab,
  NTabs,
  useMessage,
} from "naive-ui";
import { storeToRefs } from "pinia";
import { CaretDownOutline } from "@vicons/ionicons5";

import { useAPISettingStore } from "../../stores/api_setting";
import { HTTPRequest } from "../../commands/http_request";
import { showError } from "../../helpers/util";
import { i18nCollection } from "../../i18n";
import APISettingParamsURI from "./uri";

const wrapperClass = css`
  margin-left: 5px;
  .n-divider {
    margin: 0;
  }
`;

const tabClass = css`
  .n-tabs-tab__label {
    padding: 0 15px;
    .n-icon {
      margin-left: 5px;
    }
  }
`;

enum TabItem {
  Body = "Body",
  Query = "Query",
  Auth = "Auth",
  Header = "Header",
}

export default defineComponent({
  name: "APISettingParams",
  setup() {
    const message = useMessage();
    const settingStore = useAPISettingStore();
    const { selectedID } = storeToRefs(settingStore);
    const reqParams = ref({} as HTTPRequest);
    const stop = watch(selectedID, (id) => {
      if (!id) {
        return;
      }
      const data = settingStore.findByID(id);
      try {
        reqParams.value = JSON.parse(data.setting || "{}") as HTTPRequest;
      } catch (err) {
        console.error(err);
      }
    });
    onBeforeUnmount(stop);
    const update = async () => {
      const id = selectedID.value;
      if (!id) {
        message.warning(i18nCollection("shouldSelectAPISettingFirst"));
        return;
      }
      const data = settingStore.findByID(id);
      try {
        data.setting = JSON.stringify(reqParams.value);
        await settingStore.updateByID(id, data);
      } catch (err) {
        showError(message, err);
      }
    };
    const handleUpdateURI = async (data: Map<string, unknown>) => {
      Object.assign(reqParams.value, data);
      await update();
    };
    return {
      selectedID,
      reqParams,
      handleUpdateURI,
    };
  },
  render() {
    const { reqParams, handleUpdateURI, selectedID } = this;

    const tabs = [
      TabItem.Body,
      TabItem.Query,
      TabItem.Header,
      TabItem.Auth,
    ].map((item) => {
      return <NTab name={item}>{item}</NTab>;
    });

    return (
      <div class={wrapperClass} key={selectedID}>
        <APISettingParamsURI
          method={reqParams.method}
          uri={reqParams.uri}
          onUpdate={(data) => {
            console.dir(data);
            handleUpdateURI(data);
          }}
        />
        <NDivider />
        <NTabs type="line" class={tabClass} defaultValue={TabItem.Query}>
          {tabs}
          {/* <NTab name={"query"}>
            <NDropdown
              options={[
                {
                  label: "abc",
                  key: "abc",
                },
              ]}
            >
              <span>
                Query
                <NIcon>
                  <CaretDownOutline />
                </NIcon>
              </span>
            </NDropdown>
          </NTab> */}
        </NTabs>
      </div>
    );
  },
});

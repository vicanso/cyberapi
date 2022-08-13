import { defineComponent, watch, ref, onBeforeUnmount } from "vue";
import { css } from "@linaria/core";
import { NDivider, useMessage } from "naive-ui";
import { storeToRefs } from "pinia";
import { debounce } from "lodash-es";

import { useAPISettingStore } from "../../stores/api_setting";
import {
  doHTTPRequest,
  getResponseBody,
  HTTPRequest,
} from "../../commands/http_request";
import { showError } from "../../helpers/util";
import { i18nCollection, i18nEnvironment } from "../../i18n";
import APISettingParamsURI from "./uri";
import APISettingParamsReqParams from "./req_params";
import { ENVRegexp, useEnvironmentStore } from "../../stores/environment";

const wrapperClass = css`
  height: 100%;
  position: relative;
  margin-left: 5px;
  .n-divider {
    margin: 0;
  }
`;

export default defineComponent({
  name: "APISettingParams",
  setup() {
    const message = useMessage();
    const settingStore = useAPISettingStore();
    const environmentStore = useEnvironmentStore();
    const { selectedID } = storeToRefs(settingStore);
    const reqParams = ref({} as HTTPRequest);
    const stop = watch(selectedID, (id) => {
      if (!id) {
        return;
      }
      try {
        reqParams.value = settingStore.getHTTPRequest(id);
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

    const handleUpdateBody = async (
      id: string,
      params: {
        body: string;
        contentType: string;
      }
    ) => {
      // 因为是延时执行，如果已经切换，则不更新
      // 避免更新了其它接口的数据
      if (id !== selectedID.value) {
        return;
      }
      reqParams.value.contentType = params.contentType;
      reqParams.value.body = params.body;
      await update();
    };

    const handleSend = async () => {
      try {
        const req = settingStore.getHTTPRequest(selectedID.value);
        if (!req.uri) {
          throw new Error(i18nEnvironment("uriIsNil"));
        }
        const arr = ENVRegexp.exec(req.uri);
        if (arr?.length === 2) {
          const envValue = environmentStore.getValue(arr[1]);
          if (envValue) {
            req.uri = req.uri.replace(arr[0], envValue);
          }
        }
        const res = await doHTTPRequest(req);
        const body = getResponseBody(res);
        console.dir(body);
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      selectedID,
      reqParams,
      handleSend,
      handleUpdateBody: debounce(handleUpdateBody, 1000),
      handleUpdateURI,
    };
  },
  render() {
    const { reqParams, selectedID } = this;

    return (
      <div class={wrapperClass} key={selectedID}>
        <APISettingParamsURI
          params={reqParams}
          onUpdateURI={(data) => {
            this.handleUpdateURI(data);
          }}
          onSumbit={() => {
            this.handleSend();
          }}
        />
        <NDivider />
        <APISettingParamsReqParams
          params={reqParams}
          onUpdateBody={(value) => {
            this.handleUpdateBody(selectedID, value);
          }}
        />
      </div>
    );
  },
});

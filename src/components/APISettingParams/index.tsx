import { defineComponent, watch, ref, onBeforeUnmount, PropType } from "vue";
import { css } from "@linaria/core";
import { NDivider, useMessage } from "naive-ui";
import { storeToRefs } from "pinia";
import { debounce } from "lodash-es";

import { useAPISettingStore } from "../../stores/api_setting";
import { HTTPRequest } from "../../commands/http_request";
import { showError } from "../../helpers/util";
import { i18nCollection } from "../../i18n";
import APISettingParamsURI from "./uri";
import APISettingParamsReqParams from "./req_params";
import { KVParam } from "../../commands/interface";

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
  props: {
    onSend: {
      type: Function as PropType<(id: string) => Promise<void>>,
      required: true,
    },
  },
  setup() {
    const message = useMessage();
    const settingStore = useAPISettingStore();
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

    const handleUpdateQuery = async (id: string, query: KVParam[]) => {
      // 因为是延时执行，如果已经切换，则不更新
      // 避免更新了其它接口的数据
      if (id !== selectedID.value) {
        return;
      }
      reqParams.value.query = query;
      await update();
    };

    const handleUpdateHeaders = async (id: string, headers: KVParam[]) => {
      // 因为是延时执行，如果已经切换，则不更新
      // 避免更新了其它接口的数据
      if (id !== selectedID.value) {
        return;
      }
      reqParams.value.headers = headers;
      await update();
    };

    return {
      selectedID,
      reqParams,
      // 避免频繁重复触发，不能设置过长
      // 如果设置过长容易导致更新了还没生效
      handleUpdateBody: debounce(handleUpdateBody, 300),
      handleUpdateURI,
      handleUpdateQuery: debounce(handleUpdateQuery, 300),
      handleUpdateHeaders: debounce(handleUpdateHeaders, 300),
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
            return this.$props.onSend(selectedID);
          }}
        />
        <NDivider />
        <APISettingParamsReqParams
          id={selectedID}
          params={reqParams}
          onUpdateBody={(value) => {
            this.handleUpdateBody(selectedID, value);
          }}
          onUpdateQuery={(value) => {
            this.handleUpdateQuery(selectedID, value);
          }}
          onUpdateHeaders={(value) => {
            this.handleUpdateHeaders(selectedID, value);
          }}
        />
      </div>
    );
  },
});

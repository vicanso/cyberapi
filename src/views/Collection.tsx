import { useMessage } from "naive-ui";
import {
  defineComponent,
  onBeforeMount,
  onBeforeUnmount,
  ref,
  watch,
} from "vue";
import { useRoute } from "vue-router";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";
import { ulid } from "ulid";

import { getBodyWidth, showError } from "../helpers/util";
import { useAPICollectionStore } from "../stores/api_collection";
import ExLoading from "../components/ExLoading";
import { useHeaderStore } from "../stores/header";
import { useSettingStore } from "../stores/setting";
import { mainHeaderHeight } from "../constants/style";
import ExColumn from "../components/ExColumn";
import APISettingTree from "../components/APISettingTree";
import APISettingParams from "../components/APISettingParams";
import { useEnvironmentStore } from "../stores/environment";
import { useAPISettingStore } from "../stores/api_setting";
import { abortRequestID, doHTTPRequest } from "../commands/http_request";
import { HTTPResponse } from "../commands/http_response";
import APIResponse from "../components/APIResponse";
import { usePinRequestStore } from "../stores/pin_request";
import { useAPIFolderStore } from "../stores/api_folder";

const contentClass = css`
  position: fixed;
  left: 0;
  right: 0;
  top: ${mainHeaderHeight + 2}px;
  bottom: 0;
`;

export default defineComponent({
  name: "CollectionView",
  setup() {
    const route = useRoute();
    const collection = route.query.id as string;
    const message = useMessage();
    const headerStore = useHeaderStore();
    const settingStore = useSettingStore();
    const apiFolderStore = useAPIFolderStore();
    const apiSettingStore = useAPISettingStore();
    const { collectionColumnWidths } = storeToRefs(settingStore);

    const processing = ref(false);
    const sending = ref(false);
    const response = ref({} as HTTPResponse);

    const stop = watch(
      () => apiSettingStore.selectedID,
      (id) => {
        // 如果选择新的api，则重置数据
        response.value = {
          api: id,
        } as HTTPResponse;
      }
    );

    onBeforeUnmount(() => {
      stop();
    });

    onBeforeMount(async () => {
      processing.value = true;
      try {
        await apiFolderStore.fetch(collection);
        await apiSettingStore.fetch(collection);
        await usePinRequestStore().fetch(collection);
        await useEnvironmentStore().fetch(collection);
        const collectionStore = useAPICollectionStore();
        const result = await collectionStore.get(collection);
        if (result) {
          headerStore.add({
            name: result.name,
            route: route.name as string,
          });
        }
        await collectionStore.fetchExpandedFolders(collection);
        await collectionStore.fetchTopTreeItems(collection);
        await collectionStore.fetchActiveTabs();
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    });

    const updateCollectionColumnWidths = async (params: {
      restWidth: number;
      value: number;
      index: number;
    }) => {
      const { index, value, restWidth } = params;
      if (index < 1 || index > 2) {
        return;
      }
      const widths = settingStore.collectionColumnWidths.slice(0);

      // 第一行绝对值，其它记录百分比
      if (index === 1) {
        widths[index - 1] += value;
      } else {
        widths[index - 1] += value / restWidth;
      }
      try {
        await settingStore.updateCollectionColumnWidths(widths);
      } catch (err) {
        showError(message, err);
      }
    };
    let senddingRequestID = "";

    const isCurrnetRequest = (reqID: string) => {
      return senddingRequestID === reqID;
    };

    const handleSend = async (id: string) => {
      // 中断请求
      if (id === abortRequestID) {
        sending.value = false;
        senddingRequestID = "";
        response.value = {} as HTTPResponse;
        return;
      }
      if (sending.value) {
        return;
      }
      const reqID = ulid();
      senddingRequestID = reqID;

      try {
        response.value = {
          status: -1,
        } as HTTPResponse;
        sending.value = true;
        const req = apiSettingStore.getHTTPRequestFillENV(id);
        const res = await doHTTPRequest(id, req);
        if (isCurrnetRequest(reqID)) {
          response.value = res;
        }
      } catch (err) {
        if (isCurrnetRequest(reqID)) {
          response.value = {} as HTTPResponse;
          showError(message, err);
        }
      } finally {
        if (isCurrnetRequest(reqID)) {
          sending.value = false;
        }
      }
    };

    return {
      response,
      sending,
      collectionColumnWidths,
      processing,
      updateCollectionColumnWidths,
      handleSend,
    };
  },
  render() {
    const {
      processing,
      collectionColumnWidths,
      updateCollectionColumnWidths,
      response,
    } = this;
    if (processing) {
      return <ExLoading />;
    }

    let currentWidth = 0;
    const widths = collectionColumnWidths.slice(0);
    // 最后一个分栏自动适应
    if (widths.length) {
      widths.push(0);
    }
    let restWidth = getBodyWidth();
    widths.forEach((width) => {
      // 绝对值
      if (width > 1) {
        restWidth = restWidth - width;
      }
    });

    const columns = widths.map((width, index) => {
      if (width < 1) {
        width = Math.floor(restWidth * width);
      }
      let element = <div />;
      if (index === 0) {
        element = <APISettingTree />;
      } else if (index === 1) {
        element = (
          <APISettingParams
            onSend={(id) => {
              return this.handleSend(id);
            }}
          />
        );
      } else if (index === 2) {
        element = <APIResponse response={response} />;
      }
      const column = (
        <ExColumn
          left={currentWidth}
          width={width}
          showDivider={index !== 0}
          onResize={(value) => {
            // TODO 是否设置为百分比更合理
            updateCollectionColumnWidths({
              restWidth,
              value,
              index,
            });
          }}
        >
          {element}
        </ExColumn>
      );
      currentWidth += width;
      return column;
    });

    return <div class={contentClass}>{columns}</div>;
  },
});

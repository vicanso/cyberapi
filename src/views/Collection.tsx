import { useMessage } from "naive-ui";
import { defineComponent, onBeforeMount, ref } from "vue";
import { useRoute } from "vue-router";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";

import { showError } from "../helpers/util";
import { useAPICollectionStore } from "../stores/api_collection";
import ExLoading from "../components/ExLoading";
import { useHeaderStore } from "../stores/header";
import { useSettingStore } from "../stores/setting";
import { mainHeaderHeight } from "../constants/style";
import ExColumn from "../components/ExColumn";
import APISettingTree from "../components/APISettingTree";
import APISettingParams from "../components/APISettingParams";
import { ENVRegexp, useEnvironmentStore } from "../stores/environment";
import { i18nEnvironment } from "../i18n";
import { useAPISettingStore } from "../stores/api_setting";
import { doHTTPRequest, HTTPResponse } from "../commands/http_request";
import APIResponse from "../components/APIResponse";

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
    const apiSettingStore = useAPISettingStore();
    const environmentStore = useEnvironmentStore();
    const { collectionColumnWidths } = storeToRefs(settingStore);

    const processing = ref(false);
    const sending = ref(false);
    const response = ref({} as HTTPResponse);

    onBeforeMount(async () => {
      processing.value = true;
      try {
        const environmentStore = useEnvironmentStore();
        await environmentStore.fetch(collection);
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

    const updateCollectionColumnWidths = async (
      value: number,
      index: number
    ) => {
      const widths = settingStore.collectionColumnWidths.slice(0);
      widths[index - 1] += value;
      if (widths.length > index) {
        widths[index] -= value;
      }
      try {
        await settingStore.updateCollectionColumnWidths(widths);
      } catch (err) {
        showError(message, err);
      }
    };
    const handleSend = async (id: string) => {
      if (sending.value) {
        return;
      }
      try {
        response.value = {} as HTTPResponse;
        sending.value = true;
        const req = apiSettingStore.getHTTPRequest(id);
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
        response.value = res;
      } catch (err) {
        showError(message, err);
      } finally {
        sending.value = false;
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
    const columns = widths.map((width, index) => {
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
            updateCollectionColumnWidths(value, index);
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

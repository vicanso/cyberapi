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
import { useEnvironmentStore } from "../stores/environment";

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
    const { collectionColumnWidths } = storeToRefs(settingStore);

    const processing = ref(false);

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
    return {
      collectionColumnWidths,
      processing,
      updateCollectionColumnWidths,
    };
  },
  render() {
    const { processing, collectionColumnWidths, updateCollectionColumnWidths } =
      this;
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
        element = <APISettingParams />;
      }
      const column = (
        <ExColumn
          left={currentWidth}
          width={width}
          showDivider={index !== 0}
          onResize={(value) => {
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

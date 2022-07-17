import { NDivider, useMessage } from "naive-ui";
import { defineComponent, onBeforeMount, ref, StyleValue } from "vue";
import { useRoute } from "vue-router";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";

import { showError } from "../helpers/util";
import { useAPICollectionsStore } from "../stores/api_collection";
import ExLoading from "../components/ExLoading";
import { useHeaderStore } from "../stores/header";
import { APICollection } from "../commands/api_collection";
import { useSettingStore } from "../stores/setting";
import { mainHeaderHeight } from "../constants/style";
import ExColumn from "../components/ExColumn";

const contentClass = css`
  position: fixed;
  left: 0;
  right: 0;
  top: ${mainHeaderHeight + 2}px;
  bottom: 0;
`;

const dividerClass = css`
  margin: 0 !important;
  height: 100% !important;
`;

export default defineComponent({
  name: "CollectionView",
  setup() {
    const route = useRoute();
    const message = useMessage();
    const headerStore = useHeaderStore();
    const settingStore = useSettingStore();
    const collection = ref({} as APICollection);
    const { collectionColumnWidths } = storeToRefs(settingStore);

    const processing = ref(false);

    onBeforeMount(async () => {
      processing.value = true;
      try {
        const collectionStore = useAPICollectionsStore();
        const result = await collectionStore.get(route.query.id as string);
        if (result) {
          headerStore.add({
            name: result.name,
            route: route.name as string,
          });
          collection.value = result;
        }
      } catch (err) {
        showError(message, err);
      } finally {
        processing.value = false;
      }
    });
    return {
      collection,
      collectionColumnWidths,
      processing,
    };
  },
  render() {
    const { processing, collection, collectionColumnWidths } = this;
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
      const column = (
        <ExColumn left={currentWidth} width={width} showDivider={index !== 0}>
          {index}
        </ExColumn>
      );
      currentWidth += width;
      return column;
    });

    console.dir(collection);
    return <div class={contentClass}>{columns}</div>;
  },
});

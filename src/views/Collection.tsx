import { useMessage } from "naive-ui";
import { defineComponent, onBeforeMount, ref } from "vue";
import { useRoute } from "vue-router";
import { storeToRefs } from "pinia";
import { css } from "@linaria/core";

import { showError } from "../helpers/util";
import { useAPICollectionsStore } from "../stores/api_collection";
import ExLoading from "../components/ExLoading";
import { useHeaderStore } from "../stores/header";
import { useSettingStore } from "../stores/setting";
import { mainHeaderHeight } from "../constants/style";
import ExColumn from "../components/ExColumn";
import APISettingTrees from "../components/APISettingTrees";
import { useAPIFoldersStore } from "../stores/api_folder";

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
    const collectionID = route.query.id as string;
    const message = useMessage();
    const headerStore = useHeaderStore();
    const settingStore = useSettingStore();
    const folderStore = useAPIFoldersStore();
    const { collectionColumnWidths } = storeToRefs(settingStore);
    const { apiFolders } = storeToRefs(folderStore);

    const processing = ref(false);

    onBeforeMount(async () => {
      processing.value = true;
      try {
        const collectionStore = useAPICollectionsStore();
        const result = await collectionStore.get(collectionID);
        if (result) {
          headerStore.add({
            name: result.name,
            route: route.name as string,
          });
        }
        await folderStore.fetch();
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
      apiFolders,
      updateCollectionColumnWidths,
    };
  },
  render() {
    const {
      processing,
      apiFolders,
      collectionColumnWidths,
      updateCollectionColumnWidths,
    } = this;
    if (processing) {
      return <ExLoading />;
    }
    console.dir(apiFolders);

    let currentWidth = 0;
    const widths = collectionColumnWidths.slice(0);
    // 最后一个分栏自动适应
    if (widths.length) {
      widths.push(0);
    }
    const columns = widths.map((width, index) => {
      let element = <div />;
      if (index === 0) {
        element = <APISettingTrees />;
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

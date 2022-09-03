import { css } from "@linaria/core";
import { TrashOutline } from "@vicons/ionicons5";
import {
  DataTableColumns,
  NButton,
  NCard,
  NDataTable,
  NIcon,
  NPopconfirm,
  useMessage,
} from "naive-ui";
import { defineComponent } from "vue";
import { reload, showError } from "../helpers/util";

import { i18nStore } from "../i18n";
import { clearStore, StoreKey } from "../stores/local";

const storeClass = css`
  max-width: 800px;
  .n-card__content {
    min-height: 200px;
    max-height: 400px;
    overflow-y: auto;
  }
`;

export default defineComponent({
  name: "StoreSetting",
  setup() {
    const message = useMessage();
    const stores = [
      {
        name: i18nStore("settingStore"),
        desc: i18nStore("settingStoreDesc"),
        id: StoreKey.setting,
      },
      {
        name: i18nStore("pinRequestsStore"),
        desc: i18nStore("pinRequestsStoreDesc"),
        id: StoreKey.pinRequests,
      },
      {
        name: i18nStore("latestResponseStore"),
        desc: i18nStore("latestResponseStoreDesc"),
        id: StoreKey.latestResponse,
      },
    ];
    const clear = async (index: number) => {
      try {
        const { id } = stores[index];
        await clearStore(id);
        message.info(i18nStore("clearSuccess"));
        setTimeout(() => {
          reload();
        }, 3000);
      } catch (err) {
        showError(message, err);
      }
    };
    return {
      stores,
      clear,
    };
  },
  render() {
    const { stores } = this;
    const columns: DataTableColumns = [
      {
        title: i18nStore("name"),
        key: "name",
      },
      {
        title: i18nStore("desc"),
        key: "desc",
      },
      {
        title: i18nStore("op"),
        key: "op",
        width: 120,
        render: (row, index) => {
          const slots = {
            trigger: () => (
              <NButton>
                <NIcon>
                  <TrashOutline />
                </NIcon>
              </NButton>
            ),
          };
          const name = stores[index].name;
          const tips = i18nStore("clearTips").replace("%s", name);
          return (
            <NPopconfirm
              v-slots={slots}
              onPositiveClick={() => {
                this.clear(index);
              }}
            >
              {tips}
            </NPopconfirm>
          );
        },
      },
    ];
    return (
      <NCard title={i18nStore("title")} class={storeClass}>
        <NDataTable data={stores} columns={columns} />
      </NCard>
    );
  },
});

import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount } from "vue";
import {
  NCard,
  useMessage,
  DataTableColumns,
  NDataTable,
  NButton,
  NIcon,
  NSpace,
} from "naive-ui";

import { useCookieStore } from "../stores/cookie";
import { showError, getNormalDialogStyle } from "../helpers/util";
import { i18nCookie } from "../i18n";
import { CreateOutline, TrashOutline } from "@vicons/ionicons5";

export default defineComponent({
  name: "CookieSetting",
  setup() {
    const message = useMessage();
    const cookieStore = useCookieStore();
    const { cookies } = storeToRefs(cookieStore);
    onBeforeMount(async () => {
      try {
        await cookieStore.fetch();
      } catch (err) {
        showError(message, err);
      }
    });

    return {
      cookies,
    };
  },
  render() {
    const { cookies } = this;
    const modalStyle = getNormalDialogStyle();

    const columns: DataTableColumns = [
      {
        title: i18nCookie("name"),
        key: "name",
      },
      {
        title: i18nCookie("value"),
        key: "value",
      },
      {
        title: i18nCookie("path"),
        key: "path",
      },
      {
        title: i18nCookie("domain"),
        key: "domain",
      },
      {
        title: i18nCookie("expires"),
        key: "expires",
      },
      {
        title: i18nCookie("op"),
        key: "op",
        render: (row) => {
          console.dir(row);
          return (
            <NSpace>
              <NButton>
                <NIcon>
                  <CreateOutline />
                </NIcon>
              </NButton>
              <NButton>
                <NIcon>
                  <TrashOutline />
                </NIcon>
              </NButton>
            </NSpace>
          );
        },
      },
    ];

    return (
      <NCard title={i18nCookie("title")} style={modalStyle}>
        <NDataTable data={cookies} columns={columns} />
      </NCard>
    );
  },
});

import { storeToRefs } from "pinia";
import { defineComponent, onBeforeMount, ref } from "vue";
import {
  NCard,
  useMessage,
  DataTableColumns,
  NDataTable,
  NButton,
  NIcon,
  NSpace,
  useDialog,
} from "naive-ui";

import { useCookieStore } from "../stores/cookie";
import { showError, getNormalDialogStyle } from "../helpers/util";
import { i18nCookie, i18nCommon } from "../i18n";
import { CreateOutline, TrashOutline } from "@vicons/ionicons5";
import { Cookie } from "../commands/cookies";
import ExCookieEditor from "../components/ExCookieEditor";

enum Mode {
  Edit = "edit",
  List = "list",
}

export default defineComponent({
  name: "CookieSetting",
  setup() {
    const message = useMessage();
    const dialog = useDialog();
    const cookieStore = useCookieStore();
    const { cookies } = storeToRefs(cookieStore);
    const mode = ref(Mode.List);
    const updatedCookie = ref({} as Cookie);
    const updateValues = ref({
      value: "",
      expires: "",
    });
    onBeforeMount(async () => {
      try {
        await cookieStore.fetch();
      } catch (err) {
        showError(message, err);
      }
    });
    const removeCookie = async (index: number) => {
      const cookie = cookieStore.cookies[index];
      const d = dialog.warning({
        title: i18nCookie("deleteCookie"),
        content: i18nCookie("deleteCookieContent"),
        positiveText: i18nCommon("confirm"),
        onPositiveClick: async () => {
          d.loading = true;
          try {
            await cookieStore.remove(cookie);
          } catch (err) {
            showError(message, err);
          } finally {
            d.loading = false;
          }
        },
      });
    };
    const editCookie = (index: number) => {
      const cookie = cookieStore.cookies[index];
      updatedCookie.value = cookie;
      mode.value = Mode.Edit;
      updateValues.value.value = cookie.value;
      updateValues.value.expires = cookie.expires;
    };

    return {
      mode,
      cookies,
      removeCookie,
      editCookie,
      updatedCookie,
      updateValues,
    };
  },
  render() {
    const { cookies, removeCookie, editCookie, mode, updatedCookie } = this;
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
        render: (row, index) => {
          return (
            <NSpace>
              <NButton
                onClick={() => {
                  editCookie(index);
                }}
              >
                <NIcon>
                  <CreateOutline />
                </NIcon>
              </NButton>
              <NButton
                onClick={() => {
                  removeCookie(index);
                }}
              >
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
        {mode === Mode.List && <NDataTable data={cookies} columns={columns} />}
        {mode === Mode.Edit && (
          <ExCookieEditor
            cookie={updatedCookie}
            onBack={() => {
              this.mode = Mode.List;
            }}
          />
        )}
      </NCard>
    );
  },
});
